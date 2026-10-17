//! Use-case boundary tests for classified payload disclosure (FR-016-006..012).
#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use sagat_application::classification::{
    ClassificationAnalysis, ClassificationInterpreter, ClassifiedNode, ClassifiedPayload,
    DisclosedLevels, ProjectionFailure, ProjectionRequest, ScalarValue, ValueKind,
};
use sagat_application::clock::Clock;
use sagat_application::error::{AppError, StoreError};
use sagat_application::identity::RequestContext;
use sagat_application::payload_disclosure::{
    DisclosureFailure, DisclosureRead, DisclosureRun, DisclosureSelector, PayloadDisclosureResult,
    PayloadDisclosureService, PayloadDisclosureStore, PayloadDisclosureUseCase, PreparedDisclosure,
    SelectedDisclosure, StoredDefinition,
};
use sagat_application::payloads::{
    PayloadProjection, PayloadProjector, PayloadStructure, StoredPayload, StoredPayloadBytes,
    UnsupportedPayloadFormat,
};
use sagat_domain::audit::{AuditAction, AuditRecord};
use sagat_domain::classification::{ClassificationRules, Level};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Principal, Role, Subject, TenantId};
use time::OffsetDateTime;

#[derive(Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }
}

#[derive(Clone)]
struct FakeStore(Arc<Mutex<State>>);

struct State {
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    current_role: Role,
    reveal_granted: bool,
    selected: SelectedDisclosure,
    audits: Vec<AuditRecord>,
    fail_audit: bool,
}

impl FakeStore {
    fn new() -> Self {
        let tenant = TenantId::generate();
        Self(Arc::new(Mutex::new(State {
            tenant,
            namespace: NamespaceId::generate(),
            run: RunId::generate(),
            current_role: Role::Developer,
            reveal_granted: false,
            selected: SelectedDisclosure {
                run_version: 7,
                workflow_version: WorkflowVersionId::generate(),
                version_number: 3,
                classification_rules: ClassificationRules::CURRENT,
                carrier: sagat_application::classification::CarrierPointer::Input,
                node_id: None,
                attempt: None,
                definition: StoredDefinition::new(br#"{"inputSchema":{}}"#.to_vec()),
                payload: StoredPayload::Inline(StoredPayloadBytes::new(
                    br#"{"name":"Ada"}"#.to_vec(),
                )),
            },
            audits: Vec::new(),
            fail_audit: false,
        })))
    }

    fn tenant(&self) -> TenantId {
        self.0.lock().expect("state").tenant
    }

    fn run(&self) -> RunId {
        self.0.lock().expect("state").run
    }
}

impl PayloadDisclosureStore for FakeStore {
    async fn disclosure_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<DisclosureRun>, StoreError> {
        let state = self.0.lock().expect("state");
        Ok(
            (state.tenant == tenant && state.run == run).then_some(DisclosureRun {
                namespace_id: state.namespace,
            }),
        )
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        let state = self.0.lock().expect("state");
        Ok(state.tenant != tenant && state.run == run)
    }

    async fn record_disclosure_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.0.lock().expect("state").audits.push(audit.clone());
        Ok(())
    }

    async fn disclose_and_audit(
        &self,
        prepared: &PreparedDisclosure,
        evaluator: &dyn sagat_application::payload_disclosure::DisclosureEvaluator,
        audits: &sagat_application::payload_disclosure::DisclosureAudits,
    ) -> Result<DisclosureRead, StoreError> {
        let mut state = self.0.lock().expect("state");
        if state.tenant != prepared.tenant_id() || state.run != prepared.run_id() {
            return Ok(DisclosureRead::Missing);
        }
        let evaluated = evaluator.evaluate(
            state.selected.clone(),
            state.current_role,
            state.reveal_granted,
        );
        match evaluated {
            Ok(decision) => {
                if let Some(kind) = decision.audit {
                    if state.fail_audit {
                        return Err(StoreError::Unavailable {
                            detail: "audit unavailable".into(),
                        });
                    }
                    state.audits.push(audits.for_kind(kind).clone());
                }
                Ok(DisclosureRead::Ready(decision.disclosure))
            }
            Err(DisclosureFailure::PiiRevealNotGranted) => {
                state.audits.push(audits.reveal_denied.clone());
                Ok(DisclosureRead::Refused(
                    DisclosureFailure::PiiRevealNotGranted,
                ))
            }
            Err(other) => Ok(DisclosureRead::Refused(other)),
        }
    }
}

#[derive(Clone)]
struct FakeInterpreter {
    state: Arc<Mutex<InterpreterState>>,
}

struct InterpreterState {
    failure: Option<ProjectionFailure>,
    disclosed: DisclosedLevels,
    requests: Vec<(bool, bool, bool, bool, bool)>,
}

impl FakeInterpreter {
    fn new(disclosed: DisclosedLevels) -> Self {
        Self {
            state: Arc::new(Mutex::new(InterpreterState {
                failure: None,
                disclosed,
                requests: Vec::new(),
            })),
        }
    }
}

impl ClassificationInterpreter for FakeInterpreter {
    fn analyze(&self, _document: &[u8]) -> ClassificationAnalysis {
        ClassificationAnalysis::default()
    }

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        let mut state = self.state.lock().expect("interpreter state");
        state.requests.push((
            request.audience.internal,
            request.audience.pii,
            request.audience.structure,
            request.reveal.granted,
            request.reveal.requested,
        ));
        if let Some(failure) = state.failure {
            return Err(failure);
        }
        Ok(ClassifiedPayload {
            tree: ClassifiedNode::Value {
                level: Level::Public,
                kind: ValueKind::String,
                value: ScalarValue::String("Ada".into()),
                value_truncated: false,
                original_length: None,
            },
            truncated: false,
            disclosed: state.disclosed,
        })
    }
}

#[derive(Clone, Copy)]
struct StructureProjector;

impl PayloadProjector for StructureProjector {
    fn project(&self, _bytes: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat> {
        Ok(PayloadProjection {
            tree: PayloadStructure::Object {
                child_count: 1,
                children: vec![PayloadStructure::String],
                omitted_children: 0,
            },
            truncated: false,
        })
    }
}

fn principal(tenant: TenantId, role: Role) -> Principal {
    Principal::new(tenant, Subject::parse("user:alice").expect("subject"), role)
}

fn context() -> RequestContext {
    RequestContext {
        trace_id: Some("trace-016".into()),
        span_id: Some("span-016".into()),
    }
}

#[tokio::test]
async fn prepare_needs_only_run_read_and_binds_the_resolved_namespace() {
    let store = FakeStore::new();
    let interpreter = FakeInterpreter::new(DisclosedLevels::PUBLIC);
    let service =
        PayloadDisclosureService::new(store.clone(), FixedClock, interpreter, StructureProjector);

    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Viewer),
            store.run(),
            DisclosureSelector::Input,
            false,
            &context(),
        )
        .await
        .expect("viewer may read the run");
    assert_eq!(
        prepared.namespace_id(),
        store.0.lock().expect("state").namespace
    );

    let denied = service
        .prepare(
            &principal(store.tenant(), Role::Worker),
            store.run(),
            DisclosureSelector::Input,
            false,
            &context(),
        )
        .await;
    assert!(matches!(denied, Err(AppError::Forbidden { .. })));
}

#[tokio::test]
async fn current_role_and_namespace_grant_build_the_effective_audience() {
    let store = FakeStore::new();
    {
        let mut state = store.0.lock().expect("state");
        state.current_role = Role::Operator;
        state.reveal_granted = true;
    }
    let interpreter = FakeInterpreter::new(DisclosedLevels::PII);
    let service = PayloadDisclosureService::new(
        store.clone(),
        FixedClock,
        interpreter.clone(),
        StructureProjector,
    );
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Operator),
            store.run(),
            DisclosureSelector::Input,
            true,
            &context(),
        )
        .await
        .expect("prepared");
    let answer = service.disclose(prepared).await.expect("disclosed");

    assert!(matches!(
        answer.result,
        PayloadDisclosureResult::Disclosed { .. }
    ));
    assert_eq!(
        answer.caller.disclosed_levels,
        DisclosedLevels::PUBLIC
            .union(DisclosedLevels::INTERNAL)
            .union(DisclosedLevels::PII)
    );
    assert!(answer.caller.reveal_granted);
    assert!(answer.caller.revealed);
    assert_eq!(
        interpreter.state.lock().expect("state").requests,
        [(true, true, true, true, true)]
    );
    let audits = &store.0.lock().expect("state").audits;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action(), AuditAction::PayloadPiiRevealed);
    assert_eq!(audits[0].reason(), Some("classified-v1:input:reveal=pii"));
}

#[tokio::test]
async fn reveal_without_an_effective_grant_is_refused_and_audited_without_projection() {
    let store = FakeStore::new();
    let interpreter = FakeInterpreter::new(DisclosedLevels::PII);
    let service = PayloadDisclosureService::new(
        store.clone(),
        FixedClock,
        interpreter.clone(),
        StructureProjector,
    );
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Developer),
            store.run(),
            DisclosureSelector::Result,
            true,
            &context(),
        )
        .await
        .expect("prepared");

    assert_eq!(
        service.disclose(prepared).await,
        Err(sagat_application::payload_disclosure::PayloadDisclosureError::PiiRevealNotGranted)
    );
    assert!(interpreter.state.lock().expect("state").requests.is_empty());
    let audits = &store.0.lock().expect("state").audits;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action(), AuditAction::PayloadRevealDenied);
    assert_eq!(audits[0].reason(), Some("permission"));
}

#[tokio::test]
async fn a_stored_grant_is_inert_when_the_current_role_cannot_read_internal_values() {
    let store = FakeStore::new();
    {
        let mut state = store.0.lock().expect("state");
        state.current_role = Role::Viewer;
        state.reveal_granted = true;
    }
    let interpreter = FakeInterpreter::new(DisclosedLevels::PII);
    let service = PayloadDisclosureService::new(
        store.clone(),
        FixedClock,
        interpreter.clone(),
        StructureProjector,
    );
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Viewer),
            store.run(),
            DisclosureSelector::Input,
            true,
            &context(),
        )
        .await
        .expect("prepared");

    assert_eq!(
        service.disclose(prepared).await,
        Err(sagat_application::payload_disclosure::PayloadDisclosureError::PiiRevealNotGranted)
    );
    assert!(interpreter.state.lock().expect("state").requests.is_empty());
}

#[tokio::test]
async fn schema_mismatch_is_structure_only_for_inspector_and_opaque_for_developer() {
    for (role, structure_only) in [(Role::Operator, true), (Role::Developer, false)] {
        let store = FakeStore::new();
        store.0.lock().expect("state").current_role = role;
        let interpreter = FakeInterpreter::new(DisclosedLevels::NONE);
        interpreter.state.lock().expect("state").failure = Some(ProjectionFailure::SchemaMismatch);
        let service = PayloadDisclosureService::new(
            store.clone(),
            FixedClock,
            interpreter,
            StructureProjector,
        );
        let prepared = service
            .prepare(
                &principal(store.tenant(), role),
                store.run(),
                DisclosureSelector::Input,
                false,
                &context(),
            )
            .await
            .expect("prepared");
        let answer = service.disclose(prepared).await.expect("safe answer");
        assert_eq!(
            matches!(answer.result, PayloadDisclosureResult::StructureOnly { .. }),
            structure_only
        );
        assert_eq!(
            matches!(answer.result, PayloadDisclosureResult::Unavailable { .. }),
            !structure_only
        );
    }
}

#[tokio::test]
async fn audit_failure_prevents_a_projected_internal_value_from_returning() {
    let store = FakeStore::new();
    store.0.lock().expect("state").fail_audit = true;
    let interpreter = FakeInterpreter::new(DisclosedLevels::INTERNAL);
    let service =
        PayloadDisclosureService::new(store.clone(), FixedClock, interpreter, StructureProjector);
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Developer),
            store.run(),
            DisclosureSelector::Input,
            false,
            &context(),
        )
        .await
        .expect("prepared");
    assert_eq!(
        service.disclose(prepared).await,
        Err(
            sagat_application::payload_disclosure::PayloadDisclosureError::Application(
                AppError::Unavailable
            )
        )
    );
}

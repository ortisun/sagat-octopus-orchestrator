//! Use-case boundary tests for classified payload disclosure (FR-016-006..012).
#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationCacheKey, ClassificationInterpreter,
    ClassifiedNode, ClassifiedPayload, DisclosedLevels, ProjectionFailure, ProjectionRequest,
    ScalarValue, ValueKind,
};
use sagat_application::clock::Clock;
use sagat_application::error::{AppError, StoreError};
use sagat_application::identity::RequestContext;
use sagat_application::payload_disclosure::{
    CarrierWarmup, DisclosureFailure, DisclosureRead, DisclosureRun, DisclosureSelector,
    PayloadDisclosureResult, PayloadDisclosureService, PayloadDisclosureStore,
    PayloadDisclosureUseCase, PreparedDisclosure, SelectedDisclosure, StoredDefinition,
    StructureOnlyReason, UnavailableDisclosureReason,
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
    lookups: u32,
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
                carrier: Some(sagat_application::classification::CarrierPointer::Input),
                node_id: None,
                attempt: None,
                definition: StoredDefinition::new(br#"{"inputSchema":{}}"#.to_vec()),
                payload: StoredPayload::Inline(StoredPayloadBytes::new(
                    br#"{"name":"Ada"}"#.to_vec(),
                )),
            },
            audits: Vec::new(),
            fail_audit: false,
            lookups: 0,
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
        let mut state = self.0.lock().expect("state");
        state.lookups += 1;
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
        // The adapter contract: prepare the carrier before anything that holds locks.
        if let Some(carrier) = &state.selected.carrier {
            evaluator.prepare(CarrierWarmup {
                workflow_version: state.selected.workflow_version,
                carrier,
                rules: state.selected.classification_rules,
                definition: state.selected.definition.as_slice(),
            });
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
    prepared: Vec<(ClassificationCacheKey, ClassificationRules, usize)>,
}

impl FakeInterpreter {
    fn new(disclosed: DisclosedLevels) -> Self {
        Self {
            state: Arc::new(Mutex::new(InterpreterState {
                failure: None,
                disclosed,
                requests: Vec::new(),
                prepared: Vec::new(),
            })),
        }
    }
}

impl ClassificationInterpreter for FakeInterpreter {
    fn analyze(&self, _document: &[u8]) -> ClassificationAnalysis {
        ClassificationAnalysis::default()
    }

    fn prepare(&self, key: &ClassificationCacheKey, rules: ClassificationRules, document: &[u8]) {
        self.state
            .lock()
            .expect("interpreter state")
            .prepared
            .push((key.clone(), rules, document.len()));
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
async fn forbidden_callers_never_lookup_and_missing_and_foreign_runs_stay_opaque() {
    let store = FakeStore::new();
    let service = PayloadDisclosureService::new(
        store.clone(),
        FixedClock,
        FakeInterpreter::new(DisclosedLevels::PUBLIC),
        StructureProjector,
    );
    for run in [store.run(), RunId::generate()] {
        let result = service
            .prepare(
                &principal(store.tenant(), Role::Worker),
                run,
                DisclosureSelector::Input,
                false,
                &context(),
            )
            .await;
        assert!(matches!(result, Err(AppError::Forbidden { .. })));
    }
    assert_eq!(store.0.lock().expect("state").lookups, 0);
    for (tenant, run) in [
        (TenantId::generate(), store.run()),
        (store.tenant(), RunId::generate()),
    ] {
        assert!(matches!(
            service
                .prepare(
                    &principal(tenant, Role::Viewer),
                    run,
                    DisclosureSelector::Input,
                    false,
                    &context()
                )
                .await,
            Err(AppError::NotFound)
        ));
    }
    assert_eq!(store.0.lock().expect("state").lookups, 2);
}

#[tokio::test]
async fn closed_projector_failures_never_authorize_values_or_emit_a_success_audit() {
    let cases = [
        (
            ProjectionFailure::LegacyVersion,
            Some(StructureOnlyReason::LegacyVersion),
        ),
        (
            ProjectionFailure::NoCarrier,
            Some(StructureOnlyReason::NoSchema),
        ),
        (
            ProjectionFailure::SchemaUnusable,
            Some(StructureOnlyReason::SchemaUnusable),
        ),
        (
            ProjectionFailure::SchemaMismatch,
            Some(StructureOnlyReason::SchemaMismatch),
        ),
        (ProjectionFailure::PayloadTooLarge, None),
        (ProjectionFailure::UnsupportedPayload, None),
    ];
    for (failure, expected_structure) in cases {
        for role in [Role::Developer, Role::Operator] {
            let store = FakeStore::new();
            store.0.lock().expect("state").current_role = role;
            let interpreter = FakeInterpreter::new(DisclosedLevels::PII);
            interpreter.state.lock().expect("interpreter").failure = Some(failure);
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
            let answer = service.disclose(prepared).await.expect("closed answer");
            if let Some(expected) = expected_structure.filter(|_| role == Role::Operator) {
                assert!(
                    matches!(answer.result, PayloadDisclosureResult::StructureOnly { reason, .. } if reason == expected)
                );
            } else {
                let expected = if expected_structure.is_some() {
                    UnavailableDisclosureReason::NotDisclosable
                } else {
                    UnavailableDisclosureReason::UnsupportedFormat
                };
                assert!(
                    matches!(answer.result, PayloadDisclosureResult::Unavailable { reason, .. } if reason == expected)
                );
            }
            assert!(store.0.lock().expect("state").audits.is_empty());
        }
    }
}

#[test]
fn selected_payload_and_definition_debug_never_print_values_keys_or_schema_fragments() {
    let store = FakeStore::new();
    let mut state = store.0.lock().expect("state");
    state.selected.definition =
        StoredDefinition::new(br#"{"debug-schema-canary":"fragment"}"#.to_vec());
    state.selected.payload = StoredPayload::Inline(StoredPayloadBytes::new(
        br#"{"debug-key-canary":"debug-value-canary"}"#.to_vec(),
    ));
    let debug = format!("{:?}", state.selected);
    assert!(debug.contains("size_bytes"));
    for forbidden in [
        "debug-schema-canary",
        "debug-key-canary",
        "debug-value-canary",
    ] {
        assert!(
            !debug.contains(forbidden),
            "Debug exposed a payload component"
        );
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
async fn a_non_carrier_journal_event_never_borrows_another_nodes_schema() {
    for (role, structure_only) in [(Role::Operator, true), (Role::Developer, false)] {
        let store = FakeStore::new();
        {
            let mut state = store.0.lock().expect("state");
            state.current_role = role;
            state.selected.carrier = None;
            state.selected.node_id = Some("same-id".into());
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
                &principal(store.tenant(), role),
                store.run(),
                DisclosureSelector::Journal { sequence: 9 },
                false,
                &context(),
            )
            .await
            .expect("prepared");
        let answer = service.disclose(prepared).await.expect("closed answer");

        assert!(
            interpreter.state.lock().expect("state").requests.is_empty(),
            "an unrecognised event type has no classification carrier"
        );
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

#[tokio::test]
async fn the_carrier_is_prepared_once_with_the_pinned_identity_and_never_for_a_legacy_version() {
    let store = FakeStore::new();
    let interpreter = FakeInterpreter::new(DisclosedLevels::PUBLIC);
    let service = PayloadDisclosureService::new(
        store.clone(),
        FixedClock,
        interpreter.clone(),
        StructureProjector,
    );
    let read = |role| {
        let service = &service;
        let store = store.clone();
        async move {
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
            service.disclose(prepared).await.expect("answer")
        }
    };
    read(Role::Viewer).await;
    let (version, definition_bytes) = {
        let state = store.0.lock().expect("state");
        (
            state.selected.workflow_version,
            state.selected.definition.as_slice().len(),
        )
    };
    assert_eq!(
        interpreter.state.lock().expect("interpreter").prepared,
        vec![(
            ClassificationCacheKey {
                version,
                carrier: CarrierPointer::Input,
            },
            ClassificationRules::CURRENT,
            definition_bytes,
        )]
    );

    store.0.lock().expect("state").selected.classification_rules = ClassificationRules::NONE;
    read(Role::Viewer).await;
    assert_eq!(
        interpreter
            .state
            .lock()
            .expect("interpreter")
            .prepared
            .len(),
        1,
        "a version without classification rules has nothing to compile"
    );
}

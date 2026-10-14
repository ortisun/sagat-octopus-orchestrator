//! Use-case boundary tests for ADR-0028.
#![allow(clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::error::{AppError, StoreError};
use sagat_application::identity::RequestContext;
use sagat_application::payloads::{
    InspectionResult, InspectionRun, MAX_INSPECTION_INPUT_BYTES, PayloadInspectionService,
    PayloadInspectionStore, PayloadInspectionUseCase, PayloadProjection, PayloadProjector,
    PayloadSelector, PayloadStructure, PreparedInspection, SelectedPayload, StoredPayload,
    StoredPayloadBytes, UnavailablePayloadReason, UnsupportedPayloadFormat,
};
use sagat_domain::audit::{AuditAction, AuditRecord};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, Subject, TenantId};
use time::OffsetDateTime;

#[derive(Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }
}

#[derive(Clone)]
struct FakeStore {
    state: Arc<Mutex<State>>,
}

struct State {
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    selected: Option<SelectedPayload>,
    fail_success_audit: bool,
    audits: Vec<AuditRecord>,
}

impl FakeStore {
    fn new() -> Self {
        let tenant = TenantId::generate();
        Self {
            state: Arc::new(Mutex::new(State {
                tenant,
                namespace: NamespaceId::generate(),
                run: RunId::generate(),
                selected: Some(SelectedPayload {
                    run_version: 7,
                    node_id: None,
                    attempt: None,
                    payload: StoredPayload::Inline(StoredPayloadBytes::new(
                        br#"{"secret":"CANARY"}"#.to_vec(),
                    )),
                }),
                fail_success_audit: false,
                audits: Vec::new(),
            })),
        }
    }

    fn tenant(&self) -> TenantId {
        self.state.lock().expect("state").tenant
    }

    fn run(&self) -> RunId {
        self.state.lock().expect("state").run
    }
}

impl PayloadInspectionStore for FakeStore {
    async fn inspection_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<InspectionRun>, StoreError> {
        let state = self.state.lock().expect("state");
        Ok(
            (state.tenant == tenant && state.run == run).then_some(InspectionRun {
                namespace_id: state.namespace,
            }),
        )
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        let state = self.state.lock().expect("state");
        Ok(state.tenant != tenant && state.run == run)
    }

    async fn record_inspection_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.state.lock().expect("state").audits.push(audit.clone());
        Ok(())
    }

    async fn inspect_and_audit(
        &self,
        _prepared: &PreparedInspection,
        audit: &AuditRecord,
    ) -> Result<Option<SelectedPayload>, StoreError> {
        let mut state = self.state.lock().expect("state");
        if state.fail_success_audit {
            return Err(StoreError::Unavailable {
                detail: "injected audit failure".to_owned(),
            });
        }
        state.audits.push(audit.clone());
        Ok(state.selected.clone())
    }
}

#[derive(Clone)]
struct CountingProjector(Arc<AtomicUsize>);

impl PayloadProjector for CountingProjector {
    fn project(&self, _bytes: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat> {
        self.0.fetch_add(1, Ordering::SeqCst);
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
    Principal::new(
        tenant,
        Subject::parse("operator@example.test").expect("subject"),
        role,
    )
}

fn context() -> RequestContext {
    RequestContext {
        trace_id: Some("a".repeat(32)),
        span_id: None,
    }
}

#[tokio::test]
async fn only_admin_and_operator_prepare_an_inspection_and_denials_are_audited() {
    for role in [Role::Admin, Role::Operator] {
        let store = FakeStore::new();
        let projector = CountingProjector(Arc::new(AtomicUsize::new(0)));
        let service = PayloadInspectionService::new(store.clone(), FixedClock, projector);
        let prepared = service
            .prepare(
                &principal(store.tenant(), role),
                store.run(),
                PayloadSelector::Input,
                &context(),
            )
            .await
            .expect("authorized");
        assert_eq!(
            prepared.namespace_id(),
            store.state.lock().expect("state").namespace
        );
    }

    for role in [Role::Developer, Role::Viewer, Role::Worker, Role::Runner] {
        let store = FakeStore::new();
        let projector = CountingProjector(Arc::new(AtomicUsize::new(0)));
        let service = PayloadInspectionService::new(store.clone(), FixedClock, projector);
        let result = service
            .prepare(
                &principal(store.tenant(), role),
                store.run(),
                PayloadSelector::Input,
                &context(),
            )
            .await;
        assert!(
            matches!(result, Err(AppError::Forbidden { .. })),
            "{role:?}"
        );
        let state = store.state.lock().expect("state");
        assert_eq!(state.audits.len(), 1);
        assert_eq!(
            state.audits[0].action(),
            AuditAction::PayloadInspectionDenied
        );
        assert_eq!(state.audits[0].reason(), Some("permission"));
    }
}

#[tokio::test]
async fn an_audit_failure_returns_unavailable_before_projection_sees_bytes() {
    let store = FakeStore::new();
    store.state.lock().expect("state").fail_success_audit = true;
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PayloadInspectionService::new(
        store.clone(),
        FixedClock,
        CountingProjector(Arc::clone(&calls)),
    );
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Operator),
            store.run(),
            PayloadSelector::Input,
            &context(),
        )
        .await
        .expect("authorized");
    assert_eq!(service.inspect(prepared).await, Err(AppError::Unavailable));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_oversized_inline_value_is_unavailable_without_reaching_the_projector() {
    let store = FakeStore::new();
    store.state.lock().expect("state").selected = Some(SelectedPayload {
        run_version: 8,
        node_id: None,
        attempt: None,
        payload: StoredPayload::Inline(StoredPayloadBytes::new(vec![
            b'x';
            MAX_INSPECTION_INPUT_BYTES + 1
        ])),
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PayloadInspectionService::new(
        store.clone(),
        FixedClock,
        CountingProjector(Arc::clone(&calls)),
    );
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Operator),
            store.run(),
            PayloadSelector::Input,
            &context(),
        )
        .await
        .expect("authorized");
    let inspection = service.inspect(prepared).await.expect("bounded response");
    assert_eq!(
        inspection.result,
        InspectionResult::Unavailable {
            reason: UnavailablePayloadReason::UnsupportedFormat,
            size_bytes: Some((MAX_INSPECTION_INPUT_BYTES + 1) as u64),
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn success_uses_the_server_selector_in_the_audit_and_returns_provenance() {
    let store = FakeStore::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PayloadInspectionService::new(
        store.clone(),
        FixedClock,
        CountingProjector(Arc::clone(&calls)),
    );
    let selector = PayloadSelector::Journal { sequence: 42 };
    let prepared = service
        .prepare(
            &principal(store.tenant(), Role::Admin),
            store.run(),
            selector,
            &context(),
        )
        .await
        .expect("authorized");
    let inspection = service.inspect(prepared).await.expect("inspection");
    assert_eq!(inspection.provenance.selection, selector);
    assert_eq!(inspection.provenance.run_version, 7);
    assert!(matches!(
        inspection.result,
        InspectionResult::InlineJson { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let state = store.state.lock().expect("state");
    assert_eq!(state.audits.len(), 1);
    assert_eq!(state.audits[0].action(), AuditAction::PayloadInspected);
    assert_eq!(
        state.audits[0].reason(),
        Some("structure-only-v1:journal:42")
    );
}

#[test]
fn raw_byte_debug_never_contains_the_payload() {
    let stored = StoredPayloadBytes::new(b"ORDINARY-SECRET-CANARY".to_vec());
    let debug = format!("{:?}", StoredPayload::Inline(stored));
    assert!(!debug.contains("ORDINARY-SECRET-CANARY"));
    assert!(debug.contains("size_bytes"));
}

#[test]
fn the_dedicated_permission_is_never_implied_for_non_operator_roles() {
    for role in [Role::Developer, Role::Viewer, Role::Worker, Role::Runner] {
        assert!(!role.grants(Permission::RunPayloadInspect));
    }
}

//! Use-case tests for workflow definitions, against an in-memory store (T002-005).
//!
//! The fake mirrors the contract of `DefinitionStore` (tenant filter, name uniqueness, revision
//! compare-and-swap, idempotency replay). It proves the *service* logic: authorization before
//! storage, tenant derivation, audit records, error mapping. SQL-level guarantees are proven
//! against a real PostgreSQL in `tests/integration/durable-execution`.
#![allow(
    clippy::expect_used,
    clippy::significant_drop_tightening,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::definitions::{
    Applied, DefinitionService, DefinitionStore, Idempotency, IrCodec, IrError, ParsedDefinition,
    StoredPlan, StoredPlanEntry, VersionRecord, WorkflowRecord,
};
use sagat_application::error::{AppError, ConflictKind, StoreError};
use sagat_application::identity::{PageRequest, RequestContext};
use sagat_application::quotas::QuotaLookup;
use sagat_application::secrets::{SecretAccessLookup, SecretState};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Permission, Principal, Role, Slug, Subject, TenantId,
    WorkspaceId,
};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::{
    ActivityName, Digest, Node, NodeId, NodeKind, RetryPolicy, WorkflowDefinition,
};
use time::OffsetDateTime;

const T0: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        T0
    }
}

#[derive(Default)]
struct State {
    namespaces: Vec<Namespace>,
    workflows: BTreeMap<WorkflowId, (Workflow, Vec<u8>)>,
    versions: Vec<(WorkflowVersion, Vec<u8>)>,
    idempotency: BTreeMap<(String, String), (Vec<u8>, String)>,
    audits: Vec<AuditRecord>,
    denials: Vec<AuditRecord>,
    calls: Vec<String>,
    /// The workflow quota the fake reports; `None` is the platform default.
    workflow_limit: Option<u64>,
    /// The secrets of the namespace and whether the publisher is on each one's access list.
    secrets: BTreeMap<String, bool>,
    /// The plans handed to `publish`, by version.
    plans: BTreeMap<WorkflowVersionId, Vec<sagat_domain::saga::SagaPlan>>,
}

#[derive(Clone, Default)]
struct FakeStore(Arc<Mutex<State>>);

impl FakeStore {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.lock().expect("lock"))
    }

    /// What publishing does to the workflow: the version becomes the active one.
    fn activate(state: &mut State, id: WorkflowId, version: WorkflowVersionId) {
        if let Some((w, _)) = state.workflows.get_mut(&id) {
            *w = Workflow::restore(
                w.id(),
                w.tenant_id(),
                w.namespace_id(),
                w.name().clone(),
                w.draft_revision(),
                Some(version),
                w.created_at(),
                w.updated_at(),
            );
        }
    }
}

impl QuotaLookup for FakeStore {
    async fn effective_limit(
        &self,
        _tenant: TenantId,
        _namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        Ok(self.with(|s| match resource {
            QuotaResource::Workflows => s.workflow_limit.unwrap_or(resource.default_limit()),
            other => other.default_limit(),
        }))
    }
}

impl sagat_application::scripts::ScriptLookup for FakeStore {
    async fn missing_artifacts(
        &self,
        _tenant: TenantId,
        digests: &[sagat_domain::scripts::Digest],
    ) -> Result<Vec<sagat_domain::scripts::Digest>, StoreError> {
        // No script was built in this fake: every pinned artifact is missing.
        Ok(digests.to_vec())
    }
}

impl SecretAccessLookup for FakeStore {
    async fn secret_states(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
        names: &[SecretName],
        _subject: &str,
    ) -> Result<Vec<(SecretName, SecretState)>, StoreError> {
        Ok(self.with(|s| {
            s.calls.push("secret_states".into());
            names
                .iter()
                .map(|name| {
                    let state = s
                        .secrets
                        .get(name.as_str())
                        .map_or(SecretState::Missing, |granted| SecretState::Present {
                            granted: *granted,
                        });
                    (name.clone(), state)
                })
                .collect()
        }))
    }
}

impl DefinitionStore for FakeStore {
    async fn find_namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> Result<Option<Namespace>, StoreError> {
        Ok(self.with(|s| {
            s.calls.push("find_namespace".into());
            s.namespaces
                .iter()
                .find(|n| n.tenant_id() == tenant && n.slug() == slug)
                .cloned()
        }))
    }

    async fn create_workflow(
        &self,
        workflow: &Workflow,
        draft: &[u8],
        audit: &AuditRecord,
        idempotency: &Idempotency,
        max_in_namespace: u64,
    ) -> Result<Applied<()>, StoreError> {
        self.with(|s| {
            s.calls.push("create_workflow".into());
            let slot = (idempotency.scope.clone(), idempotency.key.clone());
            if let Some((fingerprint, id)) = s.idempotency.get(&slot) {
                return if *fingerprint == idempotency.fingerprint {
                    Ok(Applied::Replayed(id.clone()))
                } else {
                    Err(StoreError::IdempotencyKeyReused)
                };
            }
            if s.workflows.values().any(|(w, _)| {
                w.tenant_id() == workflow.tenant_id()
                    && w.namespace_id() == workflow.namespace_id()
                    && w.name() == workflow.name()
            }) {
                return Err(StoreError::Conflict(ConflictKind::WorkflowName));
            }
            let in_namespace = s
                .workflows
                .values()
                .filter(|(w, _)| {
                    w.tenant_id() == workflow.tenant_id()
                        && w.namespace_id() == workflow.namespace_id()
                })
                .count() as u64;
            if in_namespace >= max_in_namespace {
                return Err(StoreError::QuotaExceeded {
                    what: "workflows in the namespace",
                });
            }
            s.idempotency.insert(
                slot,
                (idempotency.fingerprint.clone(), workflow.id().to_string()),
            );
            s.workflows
                .insert(workflow.id(), (workflow.clone(), draft.to_vec()));
            s.audits.push(audit.clone());
            Ok(Applied::Done(()))
        })
    }

    async fn find_workflow_by_name(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &Slug,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        let id = self.with(|s| {
            s.workflows
                .values()
                .find(|(w, _)| {
                    w.tenant_id() == tenant && w.namespace_id() == namespace && w.name() == name
                })
                .map(|(w, _)| w.id())
        });
        match id {
            Some(id) => self.get_workflow(tenant, id).await,
            None => Ok(None),
        }
    }

    async fn get_workflow(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        Ok(self.with(|s| {
            s.calls.push("get_workflow".into());
            let (w, draft) = s
                .workflows
                .get(&id)
                .filter(|(w, _)| w.tenant_id() == tenant)?;
            let active = w
                .active_version()
                .and_then(|v| s.versions.iter().find(|(x, _)| x.id() == v))
                .map(|(v, _)| v.clone());
            Some(WorkflowRecord {
                workflow: w.clone(),
                draft: draft.clone(),
                active,
            })
        }))
    }

    async fn list_workflows(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<WorkflowId>,
        limit: u32,
    ) -> Result<Vec<WorkflowRecord>, StoreError> {
        Ok(self.with(|s| {
            s.workflows
                .values()
                .filter(|(w, _)| w.tenant_id() == tenant && w.namespace_id() == namespace)
                .filter(|(w, _)| after.is_none_or(|a| w.id() > a))
                .take(limit as usize)
                .map(|(w, d)| WorkflowRecord {
                    workflow: w.clone(),
                    draft: d.clone(),
                    active: None,
                })
                .collect()
        }))
    }

    async fn update_draft(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        expected_revision: u64,
        draft: &[u8],
        audit: &AuditRecord,
        now: OffsetDateTime,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        self.with(|s| {
            s.calls.push("update_draft".into());
            let Some((w, stored)) = s
                .workflows
                .get_mut(&id)
                .filter(|(w, _)| w.tenant_id() == tenant)
            else {
                return Ok(None);
            };
            if w.draft_revision() != expected_revision {
                return Err(StoreError::RevisionConflict {
                    current: w.draft_revision(),
                });
            }
            *w = Workflow::restore(
                w.id(),
                w.tenant_id(),
                w.namespace_id(),
                w.name().clone(),
                w.draft_revision() + 1,
                w.active_version(),
                w.created_at(),
                now,
            );
            *stored = draft.to_vec();
            let record = WorkflowRecord {
                workflow: w.clone(),
                draft: stored.clone(),
                active: None,
            };
            s.audits.push(audit.clone());
            Ok(Some(record))
        })
    }

    async fn publication_policy(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
    ) -> Result<Option<sagat_domain::versioning::ApprovalPolicy>, StoreError> {
        // The fake's namespaces ask for nothing.
        Ok(Some(sagat_domain::versioning::ApprovalPolicy::NONE))
    }

    async fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        _declared_state: Option<&sagat_application::declared_operations::PublishedDeclaredState>,
        _release_notes: Option<&str>,
        plans: &[sagat_domain::saga::SagaPlan],
        _triggers: &[sagat_application::dispatch::TriggerRegistration],
        _context: &sagat_application::definitions::PublishContext,
        audit: &AuditRecord,
        idempotency: &Idempotency,
        now: OffsetDateTime,
    ) -> Result<Option<Applied<(WorkflowVersion, bool)>>, StoreError> {
        self.with(|s| {
            s.calls.push("publish".into());
            let slot = (idempotency.scope.clone(), idempotency.key.clone());
            if let Some((fingerprint, number)) = s.idempotency.get(&slot) {
                return if *fingerprint == idempotency.fingerprint {
                    Ok(Some(Applied::Replayed(number.clone())))
                } else {
                    Err(StoreError::IdempotencyKeyReused)
                };
            }
            let Some((w, _)) = s
                .workflows
                .get(&id)
                .filter(|(w, _)| w.tenant_id() == principal.tenant_id())
            else {
                return Ok(None);
            };
            if w.draft_revision() != draft_revision {
                return Err(StoreError::RevisionConflict {
                    current: w.draft_revision(),
                });
            }
            if let Some((existing, _)) = s
                .versions
                .iter()
                .find(|(v, _)| v.workflow_id() == id && *v.digest() == digest)
            {
                let existing = existing.clone();
                Self::activate(s, id, existing.id());
                return Ok(Some(Applied::Done((existing, false))));
            }
            let number = u32::try_from(
                s.versions
                    .iter()
                    .filter(|(v, _)| v.workflow_id() == id)
                    .count()
                    + 1,
            )
            .expect("small");
            let version = WorkflowVersion::publish(principal, w, version_id, number, digest, now)
                .expect("authorised");
            s.idempotency
                .insert(slot, (idempotency.fingerprint.clone(), number.to_string()));
            s.versions.push((version.clone(), document.to_vec()));
            Self::activate(s, id, version_id);
            s.plans.insert(version_id, plans.to_vec());
            s.audits.push(audit.clone());
            Ok(Some(Applied::Done((version, true))))
        })
    }

    async fn get_version(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        number: u32,
    ) -> Result<Option<VersionRecord>, StoreError> {
        Ok(self.with(|s| {
            s.versions
                .iter()
                .find(|(v, _)| {
                    v.tenant_id() == tenant && v.workflow_id() == id && v.number() == number
                })
                .map(|(v, d)| VersionRecord {
                    version: v.clone(),
                    document: d.clone(),
                    release_notes: None,
                })
        }))
    }

    async fn compensation_plans(
        &self,
        _tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Vec<StoredPlan>, StoreError> {
        Ok(self.with(|s| {
            s.plans
                .get(&version)
                .map(|plans| {
                    plans
                        .iter()
                        .map(|plan| StoredPlan {
                            saga: plan.saga.clone(),
                            on_compensation_failure: plan.on_compensation_failure,
                            cancel_policy: plan.cancel_policy,
                            irreversible: plan.irreversible.clone(),
                            entries: plan
                                .entries
                                .iter()
                                .map(|e| StoredPlanEntry {
                                    node: e.node.clone(),
                                    compensation_id: sagat_application::execution::compensation_id(
                                        version, &e.node,
                                    ),
                                    activity: e.activity.clone(),
                                    after: e.after.clone(),
                                })
                                .collect(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        }))
    }

    async fn get_version_by_id(
        &self,
        tenant: TenantId,
        id: WorkflowVersionId,
    ) -> Result<Option<VersionRecord>, StoreError> {
        Ok(self.with(|s| {
            s.versions
                .iter()
                .find(|(v, _)| v.tenant_id() == tenant && v.id() == id)
                .map(|(v, d)| VersionRecord {
                    version: v.clone(),
                    document: d.clone(),
                    release_notes: None,
                })
        }))
    }

    async fn list_versions(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        after: Option<u32>,
        limit: u32,
    ) -> Result<Vec<VersionRecord>, StoreError> {
        Ok(self.with(|s| {
            s.versions
                .iter()
                .filter(|(v, _)| v.tenant_id() == tenant && v.workflow_id() == id)
                .filter(|(v, _)| after.is_none_or(|a| v.number() > a))
                .take(limit as usize)
                .map(|(v, d)| VersionRecord {
                    version: v.clone(),
                    document: d.clone(),
                    release_notes: None,
                })
                .collect()
        }))
    }

    async fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<bool, StoreError> {
        Ok(self.with(|s| {
            s.workflows
                .get(&id)
                .is_some_and(|(w, _)| w.tenant_id() != tenant)
        }))
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.with(|s| s.denials.push(audit.clone()));
        Ok(())
    }
}

/// A codec that accepts any document and names the workflow after its first line.
#[derive(Clone)]
struct FakeCodec {
    name: &'static str,
    outcome: Result<(), IrError>,
    /// Secrets the parsed definition declares on its activity.
    secrets: &'static [&'static str],
    /// The activity sits in a saga and has a compensation (`undo.a`).
    saga: bool,
}

impl IrCodec for FakeCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        if document.len() > sagat_application::definitions::MAX_IR_BYTES {
            Err(IrError::TooLarge)
        } else if document.first() == Some(&b'{') {
            Ok(())
        } else {
            Err(IrError::Malformed)
        }
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        // Enough to see which documents the service handed over: the real comparison is the
        // adapter's (crates/infra-ir/tests/diff.rs).
        Ok(sagat_application::diff::DocumentDiff {
            identical: from == to,
            nodes_added: vec![String::from_utf8_lossy(from).into_owned()],
            nodes_removed: vec![String::from_utf8_lossy(to).into_owned()],
            ..Default::default()
        })
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        self.outcome.clone()?;
        let node = Node::new(
            NodeId::parse("a").expect("id"),
            NodeKind::Activity {
                activity: ActivityName::parse("act.a").expect("name"),
                retry: RetryPolicy::DEFAULT,
                compensate_with: self
                    .saga
                    .then(|| ActivityName::parse("undo.a").expect("name")),
                reconcile_with: None,
                idempotency_template: None,
                secrets: self
                    .secrets
                    .iter()
                    .map(|name| SecretName::parse(name).expect("secret name"))
                    .collect(),
            },
        );
        let (entry, nodes) = if self.saga {
            let saga = Node::new(
                NodeId::parse("s").expect("id"),
                NodeKind::Saga {
                    begin: NodeId::parse("a").expect("id"),
                    on_compensation_failure:
                        sagat_domain::saga::OnCompensationFailure::ManualReview,
                    cancel_policy: sagat_domain::saga::SagaCancelPolicy::CompensateCompleted,
                },
            );
            ("s", vec![saga, node])
        } else {
            ("a", vec![node])
        };
        let definition = WorkflowDefinition::new(
            Slug::parse(self.name).expect("slug"),
            NodeId::parse(entry).expect("id"),
            nodes,
        )
        .expect("valid");
        let mut digest = [0u8; 32];
        digest[0] = u8::try_from(document.len() % 251).expect("small");
        Ok(ParsedDefinition {
            definition,
            canonical: document.to_vec(),
            digest: Digest::new(digest),
            warnings: Vec::new(),
        })
    }
}

struct Fixture {
    store: FakeStore,
    service: DefinitionService<FakeStore, FixedClock, FakeCodec>,
    tenant: TenantId,
    namespace: Namespace,
}

fn principal(tenant: TenantId, role: Role) -> Principal {
    Principal::new(tenant, Subject::parse("alice").expect("subject"), role)
}

fn fixture_with(codec: FakeCodec) -> Fixture {
    let tenant = TenantId::generate();
    let namespace = Namespace::restore(
        NamespaceId::generate(),
        tenant,
        WorkspaceId::generate(),
        Slug::parse("payments").expect("slug"),
        Environment::Dev,
        T0,
    );
    let store = FakeStore::default();
    store.with(|s| s.namespaces.push(namespace.clone()));
    Fixture {
        service: DefinitionService::new(store.clone(), FixedClock, codec),
        store,
        tenant,
        namespace,
    }
}

fn fixture() -> Fixture {
    fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Ok(()),
        secrets: &[],
        saga: false,
    })
}

fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some("0af7651916cd43dd8448eb211c80319c".into()),
        span_id: None,
    }
}

const DRAFT: &[u8] = br#"{"kind":"Workflow"}"#;

async fn create(f: &Fixture, role: Role, key: &str) -> Result<WorkflowRecord, AppError> {
    f.service
        .create_workflow(
            &principal(f.tenant, role),
            "payments",
            "invoice-flow",
            DRAFT,
            key,
            &ctx(),
        )
        .await
}

// ------------------------------------------------------------------ creating

#[tokio::test]
async fn creating_a_workflow_stores_the_draft_and_audits_it() {
    let f = fixture();
    let record = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    assert_eq!(record.workflow.draft_revision(), 1);
    assert_eq!(record.workflow.tenant_id(), f.tenant);
    assert_eq!(record.workflow.namespace_id(), f.namespace.id());
    assert_eq!(record.draft, DRAFT);
    let audits = f.store.with(|s| s.audits.clone());
    assert_eq!(audits.len(), 1);
    assert_eq!(
        (
            audits[0].action(),
            audits[0].resource_type(),
            audits[0].tenant_id()
        ),
        (
            AuditAction::WorkflowCreated,
            ResourceType::Workflow,
            f.tenant
        )
    );
    assert_eq!(
        audits[0].trace_id(),
        Some("0af7651916cd43dd8448eb211c80319c")
    );
    assert_eq!(audits[0].resource_id(), record.workflow.id().to_string());
}

#[tokio::test]
async fn the_workflow_quota_in_force_is_passed_to_the_store_and_refuses_the_next_workflow() {
    let f = fixture();
    f.store.with(|s| s.workflow_limit = Some(1));
    let p = principal(f.tenant, Role::Admin);
    create(&f, Role::Admin, "key-0000001")
        .await
        .expect("the first fits");
    let second = f
        .service
        .create_workflow(&p, "payments", "another-flow", DRAFT, "key-0000002", &ctx())
        .await;
    assert_eq!(
        second.map(|_| ()),
        Err(AppError::QuotaExceeded("workflows in the namespace"))
    );
    // Repeating the request that succeeded is a replay, not a second workflow: it is not refused.
    create(&f, Role::Admin, "key-0000001")
        .await
        .expect("a replay does not count again");
}

#[tokio::test]
async fn authorization_comes_before_any_storage_access() {
    let f = fixture();
    for role in [Role::Operator, Role::Viewer, Role::Worker] {
        let denied = create(&f, role, "key-0000001").await;
        assert_eq!(
            denied.map(|_| ()),
            Err(AppError::Forbidden {
                permission: Permission::DefinitionWrite
            }),
            "{role}"
        );
    }
    assert!(
        f.store.with(|s| s.calls.is_empty()),
        "a denied caller must not reach storage"
    );
}

#[tokio::test]
async fn an_unknown_or_foreign_namespace_is_not_found() {
    let f = fixture();
    let other_tenant = principal(TenantId::generate(), Role::Admin);
    for (who, namespace) in [
        (principal(f.tenant, Role::Admin), "nowhere"),
        (other_tenant, "payments"),
        (principal(f.tenant, Role::Admin), "Bad Slug"),
    ] {
        let result = f
            .service
            .create_workflow(
                &who,
                namespace,
                "invoice-flow",
                DRAFT,
                "key-0000001",
                &ctx(),
            )
            .await;
        assert_eq!(result.map(|_| ()), Err(AppError::NotFound), "{namespace}");
    }
}

#[tokio::test]
async fn a_draft_must_be_a_json_object_of_bounded_size() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    for bad in [&b"[1]"[..], b"", b"null", b"  {}"] {
        let result = f
            .service
            .create_workflow(&p, "payments", "invoice-flow", bad, "key-0000001", &ctx())
            .await;
        assert!(
            matches!(result, Err(AppError::InvalidDefinition(_))),
            "{bad:?}"
        );
    }
    let huge = vec![b'{'; sagat_application::definitions::MAX_IR_BYTES + 1];
    let result = f
        .service
        .create_workflow(&p, "payments", "invoice-flow", &huge, "key-0000001", &ctx())
        .await;
    assert!(matches!(result, Err(AppError::InvalidDefinition(b)) if *b == IrError::TooLarge));
}

#[tokio::test]
async fn a_bad_name_is_invalid_and_a_taken_name_conflicts() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let bad = f
        .service
        .create_workflow(&p, "payments", "Not A Slug", DRAFT, "key-0000001", &ctx())
        .await;
    assert!(matches!(bad, Err(AppError::Invalid(_))));
    create(&f, Role::Admin, "key-0000001").await.expect("first");
    let again = create(&f, Role::Admin, "key-0000002").await;
    assert_eq!(
        again.map(|_| ()),
        Err(AppError::Conflict(ConflictKind::WorkflowName))
    );
}

#[tokio::test]
async fn a_retried_create_returns_the_same_workflow_and_audits_once() {
    let f = fixture();
    let first = create(&f, Role::Admin, "key-0000001").await.expect("first");
    let retry = create(&f, Role::Admin, "key-0000001").await.expect("retry");
    assert_eq!(first.workflow.id(), retry.workflow.id());
    assert_eq!(
        f.store.with(|s| (s.workflows.len(), s.audits.len())),
        (1, 1)
    );
}

#[tokio::test]
async fn the_same_key_for_a_different_request_is_refused() {
    let f = fixture();
    create(&f, Role::Admin, "key-0000001").await.expect("first");
    let other = f
        .service
        .create_workflow(
            &principal(f.tenant, Role::Admin),
            "payments",
            "invoice-flow",
            br#"{"kind":"Other"}"#,
            "key-0000001",
            &ctx(),
        )
        .await;
    assert_eq!(other.map(|_| ()), Err(AppError::IdempotencyKeyReused));
}

// ------------------------------------------------------------------ reading and isolation

#[tokio::test]
async fn reading_another_tenants_workflow_is_not_found_and_audited_in_the_actors_tenant() {
    let f = fixture();
    let mine = create(&f, Role::Admin, "key-0000001").await.expect("mine");
    let intruder = principal(TenantId::generate(), Role::Admin);
    let result = f
        .service
        .get_workflow(&intruder, mine.workflow.id(), &ctx())
        .await;
    assert_eq!(result.map(|_| ()), Err(AppError::NotFound));
    let denials = f.store.with(|s| s.denials.clone());
    assert_eq!(denials.len(), 1);
    assert_eq!(
        denials[0].tenant_id(),
        intruder.tenant_id(),
        "recorded in the actor's tenant"
    );
    assert_eq!(denials[0].action(), AuditAction::CrossTenantAccessDenied);

    // A workflow that exists nowhere leaves no trace.
    let nowhere = f
        .service
        .get_workflow(&intruder, WorkflowId::generate(), &ctx())
        .await;
    assert_eq!(nowhere.map(|_| ()), Err(AppError::NotFound));
    assert_eq!(f.store.with(|s| s.denials.len()), 1);
}

#[tokio::test]
async fn reading_needs_definition_read() {
    let f = fixture();
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    for role in [Role::Admin, Role::Developer, Role::Operator, Role::Viewer] {
        assert!(
            f.service
                .get_workflow(&principal(f.tenant, role), w.workflow.id(), &ctx())
                .await
                .is_ok(),
            "{role}"
        );
    }
    let denied = f
        .service
        .get_workflow(&principal(f.tenant, Role::Worker), w.workflow.id(), &ctx())
        .await;
    assert_eq!(
        denied.map(|_| ()),
        Err(AppError::Forbidden {
            permission: Permission::DefinitionRead
        })
    );
}

#[tokio::test]
async fn listing_is_paginated_and_scoped_to_the_namespace_and_tenant() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    for i in 0..5 {
        f.service
            .create_workflow(
                &p,
                "payments",
                &format!("flow-{i}"),
                DRAFT,
                &format!("key-000000{i}"),
                &ctx(),
            )
            .await
            .expect("created");
    }
    let page1 = f
        .service
        .list_workflows(
            &p,
            "payments",
            &PageRequest {
                limit: Some(2),
                cursor: None,
            },
        )
        .await
        .expect("page 1");
    assert_eq!(page1.items.len(), 2);
    let cursor = page1.next_cursor.clone().expect("more");
    let page2 = f
        .service
        .list_workflows(
            &p,
            "payments",
            &PageRequest {
                limit: Some(2),
                cursor: Some(cursor),
            },
        )
        .await
        .expect("page 2");
    assert_eq!(page2.items.len(), 2);
    assert!(page2.items[0].workflow.id() > page1.items[1].workflow.id());
    let outsider = f
        .service
        .list_workflows(
            &principal(TenantId::generate(), Role::Admin),
            "payments",
            &PageRequest::default(),
        )
        .await;
    assert_eq!(outsider.map(|_| ()), Err(AppError::NotFound));
    let bad_cursor = f
        .service
        .list_workflows(
            &p,
            "payments",
            &PageRequest {
                limit: None,
                cursor: Some("junk".into()),
            },
        )
        .await;
    assert!(matches!(bad_cursor, Err(AppError::Invalid(_))));
}

// ------------------------------------------------------------------ editing

#[tokio::test]
async fn editing_is_compare_and_swap_on_the_revision() {
    let f = fixture();
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let id = w.workflow.id();
    let updated = f
        .service
        .update_draft(&p, id, 1, br#"{"v":2}"#, &ctx())
        .await
        .expect("updated");
    assert_eq!(updated.workflow.draft_revision(), 2);
    let stale = f
        .service
        .update_draft(&p, id, 1, br#"{"v":3}"#, &ctx())
        .await;
    assert_eq!(
        stale.map(|_| ()),
        Err(AppError::RevisionConflict { current: 2 })
    );
    assert_eq!(
        f.store.with(|s| s
            .audits
            .iter()
            .filter(|a| a.action() == AuditAction::WorkflowDraftUpdated)
            .count()),
        1
    );
    let ghost = f
        .service
        .update_draft(&p, WorkflowId::generate(), 1, DRAFT, &ctx())
        .await;
    assert_eq!(ghost.map(|_| ()), Err(AppError::NotFound));
    let viewer = f
        .service
        .update_draft(&principal(f.tenant, Role::Viewer), id, 2, DRAFT, &ctx())
        .await;
    assert_eq!(
        viewer.map(|_| ()),
        Err(AppError::Forbidden {
            permission: Permission::DefinitionWrite
        })
    );
}

// ------------------------------------------------------------------ publishing

#[tokio::test]
async fn publishing_a_workflow_that_declares_a_secret_needs_a_publisher_who_may_use_it() {
    let f = fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Ok(()),
        secrets: &["stripe-key"],
        saga: false,
    });
    let created = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let id = created.workflow.id();
    let publish = |role: Role, key: &str| {
        let p = principal(f.tenant, role);
        let key = key.to_owned();
        let service = &f.service;
        async move {
            service
                .publish(&p, id, 1, None, &key, &ctx())
                .await
                .map(|_| ())
        }
    };
    let unusable = Err(AppError::SecretUnavailable {
        name: "stripe-key".into(),
    });

    // The secret does not exist: refused, for an administrator too.
    assert_eq!(publish(Role::Admin, "pub-0000001").await, unusable);
    assert_eq!(publish(Role::Developer, "pub-0000002").await, unusable);

    // It exists but the developer is not on its list: refused, with exactly the same answer, so a
    // publisher cannot tell a missing secret from one they may not use. The administrator is not
    // subject to the list.
    f.store.with(|s| {
        s.secrets.insert("stripe-key".into(), false);
    });
    assert_eq!(publish(Role::Developer, "pub-0000003").await, unusable);
    assert_eq!(publish(Role::Admin, "pub-0000004").await, Ok(()));

    // On the list, the developer may.
    f.store.with(|s| {
        s.secrets.insert("stripe-key".into(), true);
    });
    assert_eq!(publish(Role::Developer, "pub-0000005").await, Ok(()));

    // The refusals left a trace, recorded as denials (off the response path).
    let denials = f.store.with(|s| {
        s.denials
            .iter()
            .filter(|d| d.action() == AuditAction::SecretDenied)
            .count()
    });
    assert_eq!(denials, 3);
}

#[tokio::test]
async fn a_workflow_without_secrets_never_asks_the_store_about_them() {
    let f = fixture();
    let p = principal(f.tenant, Role::Developer);
    let created = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    f.service
        .publish(&p, created.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("published");
    let asked = f
        .store
        .with(|s| s.calls.iter().any(|c| c == "secret_states"));
    assert!(!asked, "no declaration, no lookup");
}

#[tokio::test]
async fn publishing_freezes_the_draft_as_numbered_versions() {
    let f = fixture();
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let v1 = f
        .service
        .publish(&p, w.workflow.id(), 1, Some("first"), "pub-0000001", &ctx())
        .await
        .expect("v1");
    assert!(v1.created);
    assert_eq!(v1.version.number(), 1);
    assert_eq!(v1.version.published_by().as_str(), "alice");

    f.service
        .update_draft(
            &p,
            w.workflow.id(),
            1,
            br#"{"kind":"Workflow","v":2}"#,
            &ctx(),
        )
        .await
        .expect("edit");
    let v2 = f
        .service
        .publish(&p, w.workflow.id(), 2, None, "pub-0000002", &ctx())
        .await
        .expect("v2");
    assert_eq!(v2.version.number(), 2);
    let published = f.store.with(|s| {
        s.audits
            .iter()
            .filter(|a| a.action() == AuditAction::WorkflowPublished)
            .count()
    });
    assert_eq!(published, 2);

    let frozen = f
        .service
        .get_version(&p, w.workflow.id(), 1, &ctx())
        .await
        .expect("v1 stored");
    assert_eq!(
        frozen.document, DRAFT,
        "a version keeps the document it was published from"
    );
    let listed = f
        .service
        .list_versions(&p, w.workflow.id(), &PageRequest::default(), &ctx())
        .await
        .expect("list");
    assert_eq!(
        listed
            .items
            .iter()
            .map(|r| r.version.number())
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn publishing_an_identical_document_reuses_the_version() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let first = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("first");
    let again = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000002", &ctx())
        .await
        .expect("again");
    assert!(first.created && !again.created);
    assert_eq!(first.version.id(), again.version.id());
}

#[tokio::test]
async fn a_retried_publish_replays_the_original_result() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let first = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("first");
    let retry = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("retry");
    assert_eq!(first.version.id(), retry.version.id());
    assert_eq!(f.store.with(|s| s.versions.len()), 1);
    let other = f
        .service
        .publish(
            &p,
            w.workflow.id(),
            1,
            Some("different notes"),
            "pub-0000001",
            &ctx(),
        )
        .await;
    assert_eq!(other.map(|_| ()), Err(AppError::IdempotencyKeyReused));
}

#[tokio::test]
async fn publishing_a_stale_revision_conflicts_without_validating() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    f.service
        .update_draft(&p, w.workflow.id(), 1, DRAFT, &ctx())
        .await
        .expect("edit");
    let stale = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await;
    assert_eq!(
        stale.map(|_| ()),
        Err(AppError::RevisionConflict { current: 2 })
    );
    assert_eq!(f.store.with(|s| s.versions.len()), 0);
}

#[tokio::test]
async fn publishing_needs_definition_publish() {
    let f = fixture();
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    for role in [Role::Operator, Role::Viewer, Role::Worker] {
        let denied = f
            .service
            .publish(
                &principal(f.tenant, role),
                w.workflow.id(),
                1,
                None,
                "pub-0000001",
                &ctx(),
            )
            .await;
        assert_eq!(
            denied.map(|_| ()),
            Err(AppError::Forbidden {
                permission: Permission::DefinitionPublish
            }),
            "{role}"
        );
    }
    assert!(f.store.with(|s| s.versions.is_empty()));
}

#[tokio::test]
async fn an_unpublishable_document_is_reported_and_nothing_is_stored() {
    let invalid = IrError::Unsupported {
        node: "n".into(),
        kind: "saga".into(),
        reason: "later",
    };
    let f = fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Err(invalid.clone()),
        secrets: &[],
        saga: false,
    });
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("draft accepts anything object-shaped");
    let result = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await;
    assert!(matches!(result, Err(AppError::InvalidDefinition(e)) if *e == invalid));
    assert!(
        f.store
            .with(|s| s.versions.is_empty() && !s.calls.contains(&"publish".into()))
    );
}

#[tokio::test]
async fn the_document_must_name_the_workflow_it_belongs_to() {
    let f = fixture_with(FakeCodec {
        name: "other-flow",
        outcome: Ok(()),
        secrets: &[],
        saga: false,
    });
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let result = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await;
    match result {
        Err(AppError::InvalidDefinition(e)) => assert_eq!(
            *e,
            IrError::NameMismatch {
                document: "other-flow".into(),
                workflow: "invoice-flow".into()
            }
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn release_notes_are_bounded() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let long = "n".repeat(sagat_application::definitions::MAX_RELEASE_NOTES + 1);
    let result = f
        .service
        .publish(&p, w.workflow.id(), 1, Some(&long), "pub-0000001", &ctx())
        .await;
    assert!(matches!(result, Err(AppError::NotAllowed(_))));
}

#[tokio::test]
async fn versions_of_another_tenant_are_invisible() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    f.service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("published");
    let intruder = principal(TenantId::generate(), Role::Admin);
    assert_eq!(
        f.service
            .get_version(&intruder, w.workflow.id(), 1, &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        f.service
            .list_versions(&intruder, w.workflow.id(), &PageRequest::default(), &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        f.store.with(|s| s.denials.len()),
        2,
        "both probes were audited"
    );
}

#[tokio::test]
async fn a_stale_revision_is_reported_before_the_document_is_validated() {
    // The draft is also unpublishable, but the caller is working from an outdated revision: that is
    // what they must hear first, since validating a document they no longer have is useless.
    let invalid = IrError::Malformed;
    let f = fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Err(invalid),
        secrets: &[],
        saga: false,
    });
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    f.service
        .update_draft(&p, w.workflow.id(), 1, DRAFT, &ctx())
        .await
        .expect("edit");
    let result = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(AppError::RevisionConflict { current: 2 })
    );
}

// ------------------------------------------------------------------ sagas (feature 003)

fn saga_fixture() -> Fixture {
    fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Ok(()),
        secrets: &[],
        saga: true,
    })
}

#[tokio::test]
async fn publishing_a_saga_hands_its_compensation_plan_to_the_store_with_the_version() {
    let f = saga_fixture();
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let published = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("published");

    let stored = f
        .store
        .with(|s| s.plans.get(&published.version.id()).cloned());
    let expected = sagat_domain::saga::compensation_plans(&saga_fixture_definition());
    assert_eq!(
        stored,
        Some(expected),
        "the store gets exactly what the definition derives"
    );
    assert_eq!(stored.as_ref().map(Vec::len), Some(1));

    // And a reader gets them back, with the durable id of each compensation.
    let plans = f
        .service
        .compensation_plans(&p, w.workflow.id(), 1, &ctx())
        .await
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].saga.as_str(), "s");
    assert_eq!(plans[0].entries.len(), 1);
    assert_eq!(plans[0].entries[0].activity.as_str(), "undo.a");
    assert_eq!(
        plans[0].entries[0].compensation_id,
        sagat_application::execution::compensation_id(
            published.version.id(),
            &NodeId::parse("a").expect("id")
        )
    );
}

fn saga_fixture_definition() -> WorkflowDefinition {
    FakeCodec {
        name: "invoice-flow",
        outcome: Ok(()),
        secrets: &[],
        saga: true,
    }
    .parse(DRAFT)
    .expect("parses")
    .definition
}

#[tokio::test]
async fn a_workflow_without_a_saga_is_published_with_no_plan() {
    let f = fixture();
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let published = f
        .service
        .publish(&p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("published");
    assert_eq!(
        f.store
            .with(|s| s.plans.get(&published.version.id()).cloned()),
        Some(Vec::new())
    );
    let plans = f
        .service
        .compensation_plans(&p, w.workflow.id(), 1, &ctx())
        .await
        .expect("plans");
    assert!(plans.is_empty());
}

#[tokio::test]
async fn reading_plans_needs_definition_read_and_a_version_that_exists() {
    let f = saga_fixture();
    let developer = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    f.service
        .publish(&developer, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("published");

    // A worker identity has no business reading definitions.
    let worker = principal(f.tenant, Role::Worker);
    let refused = f
        .service
        .compensation_plans(&worker, w.workflow.id(), 1, &ctx())
        .await;
    assert!(
        matches!(
            refused,
            Err(AppError::Forbidden {
                permission: Permission::DefinitionRead
            })
        ),
        "{refused:?}"
    );
    let missing = f
        .service
        .compensation_plans(&developer, w.workflow.id(), 9, &ctx())
        .await;
    assert!(matches!(missing, Err(AppError::NotFound)), "{missing:?}");

    // Another tenant cannot read them by guessing the ids.
    let stranger = principal(TenantId::generate(), Role::Admin);
    let hidden = f
        .service
        .compensation_plans(&stranger, w.workflow.id(), 1, &ctx())
        .await;
    assert!(matches!(hidden, Err(AppError::NotFound)), "{hidden:?}");
}

// ------------------------------------------------------------------ validating (dry run)

use sagat_application::definitions::{DiffRef, Severity, problems_of};
use sagat_application::diff::DiffSide;
use sagat_domain::workflow::WorkflowError;

#[tokio::test]
async fn validating_needs_definition_write_and_asks_the_store_for_nothing_before() {
    let f = fixture();
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    f.store.with(|s| s.calls.clear());
    let viewer = principal(f.tenant, Role::Viewer);
    let result = f
        .service
        .validate(&viewer, w.workflow.id(), None, &ctx())
        .await;
    assert!(
        matches!(result, Err(AppError::Forbidden { .. })),
        "{result:?}"
    );
    assert!(f.store.with(|s| s.calls.is_empty()));
}

#[tokio::test]
async fn a_document_that_does_not_validate_is_an_answer_with_problems_and_stores_nothing() {
    let f = fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Err(IrError::Invalid(WorkflowError::Cycle(
            NodeId::parse("loop").expect("id"),
        ))),
        secrets: &[],
        saga: false,
    });
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let audits_before = f.store.with(|s| s.audits.len());
    let validation = f
        .service
        .validate(&p, w.workflow.id(), Some(DRAFT), &ctx())
        .await
        .expect("an invalid document is not an error");
    assert!(!validation.is_valid());
    assert_eq!(validation.digest, None);
    assert_eq!(validation.problems.len(), 1);
    let problem = &validation.problems[0];
    assert_eq!(
        (problem.severity, problem.code, problem.node.as_deref()),
        (Severity::Error, "CYCLE", Some("loop"))
    );
    // Nothing was written, nothing was audited, the draft is where it was.
    assert_eq!(f.store.with(|s| s.audits.len()), audits_before);
    assert!(f.store.with(|s| s.versions.is_empty()));
    let draft = f
        .service
        .get_workflow(&p, w.workflow.id(), &ctx())
        .await
        .expect("read");
    assert_eq!(draft.workflow.draft_revision(), 1);
}

#[tokio::test]
async fn a_valid_document_gets_its_digest_and_the_warnings_publish_would_return() {
    let f = fixture();
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let validation = f
        .service
        .validate(&p, w.workflow.id(), None, &ctx())
        .await
        .expect("validated");
    assert!(validation.is_valid(), "{validation:?}");
    assert_eq!(validation.problems, []);
    assert!(validation.digest.is_some());
}

#[tokio::test]
async fn a_document_that_names_another_workflow_is_a_problem() {
    let f = fixture_with(FakeCodec {
        name: "other-flow",
        outcome: Ok(()),
        secrets: &[],
        saga: false,
    });
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let validation = f
        .service
        .validate(&p, w.workflow.id(), None, &ctx())
        .await
        .expect("validated");
    assert_eq!(
        validation
            .problems
            .iter()
            .map(|p| p.code)
            .collect::<Vec<_>>(),
        ["NAME_MISMATCH"]
    );
    assert_eq!(validation.digest, None);
}

#[tokio::test]
async fn a_secret_the_author_may_not_use_is_a_problem_on_its_node_and_leaves_no_denial() {
    let f = fixture_with(FakeCodec {
        name: "invoice-flow",
        outcome: Ok(()),
        secrets: &["db-password"],
        saga: false,
    });
    f.store.with(|s| {
        s.secrets.insert("db-password".into(), false);
    });
    let p = principal(f.tenant, Role::Developer);
    let w = create(&f, Role::Developer, "key-0000001")
        .await
        .expect("created");
    let validation = f
        .service
        .validate(&p, w.workflow.id(), None, &ctx())
        .await
        .expect("validated");
    assert_eq!(
        validation
            .problems
            .iter()
            .map(|p| (p.code, p.node.as_deref()))
            .collect::<Vec<_>>(),
        [("SECRET_UNAVAILABLE", Some("a"))]
    );
    // Publishing records a denial; a dry run does not.
    assert!(f.store.with(|s| s.denials.is_empty()));
}

#[tokio::test]
async fn validating_another_tenants_workflow_is_not_found() {
    let f = fixture();
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let stranger = principal(TenantId::generate(), Role::Admin);
    let result = f
        .service
        .validate(&stranger, w.workflow.id(), None, &ctx())
        .await;
    assert!(matches!(result, Err(AppError::NotFound)), "{result:?}");
}

#[test]
fn every_error_of_the_codec_maps_to_a_stable_code_that_says_where() {
    use sagat_application::definitions::IrIssue;
    let id = |text: &str| NodeId::parse(text).expect("id");
    // What each error should become: the code, the node, the path.
    type Expected<'a> = Vec<(&'a str, Option<&'a str>, Option<&'a str>)>;
    let table: Vec<(IrError, Expected)> = vec![
        (IrError::TooLarge, vec![("DOCUMENT_TOO_LARGE", None, None)]),
        (IrError::Malformed, vec![("DOCUMENT_MALFORMED", None, None)]),
        (
            IrError::Schema(vec![
                IrIssue {
                    path: "/nodes/2/next".into(),
                    message: "x".into(),
                },
                IrIssue {
                    path: "".into(),
                    message: "y".into(),
                },
            ]),
            vec![
                ("SCHEMA", None, Some("/nodes/2/next")),
                ("SCHEMA", None, Some("")),
            ],
        ),
        (
            IrError::Unsupported {
                node: "h".into(),
                kind: "humanApproval".into(),
                reason: "later",
            },
            vec![("UNSUPPORTED_NODE", Some("h"), None)],
        ),
        (
            IrError::Expression {
                node: "c".into(),
                field: "expression",
                message: "bad".into(),
            },
            vec![("EXPRESSION", Some("c"), None)],
        ),
        (
            IrError::NameMismatch {
                document: "a".into(),
                workflow: "b".into(),
            },
            vec![("NAME_MISMATCH", None, None)],
        ),
        (
            IrError::UnknownScript { digest: "d".into() },
            vec![("UNKNOWN_SCRIPT", None, None)],
        ),
        (
            IrError::Invalid(WorkflowError::UnknownTarget {
                from: id("a"),
                to: id("zzz"),
            }),
            vec![("UNKNOWN_TARGET", Some("a"), None)],
        ),
        (
            IrError::Invalid(WorkflowError::Unreachable(id("orphan"))),
            vec![("UNREACHABLE", Some("orphan"), None)],
        ),
        (
            IrError::Invalid(WorkflowError::NoNodes),
            vec![("NO_NODES", None, None)],
        ),
    ];
    for (error, expected) in table {
        let got: Vec<_> = problems_of(&error)
            .iter()
            .map(|p| (p.code, p.node.clone(), p.path.clone()))
            .collect();
        let want: Vec<_> = expected
            .iter()
            .map(|(c, n, p)| (*c, n.map(str::to_owned), p.map(str::to_owned)))
            .collect();
        assert_eq!(got, want, "{error:?}");
        assert!(
            problems_of(&error)
                .iter()
                .all(|p| p.severity == Severity::Error)
        );
    }
}

// ------------------------------------------------------------------ diffing

async fn published_twice(f: &Fixture, p: &Principal) -> WorkflowId {
    let w = create(f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    f.service
        .publish(p, w.workflow.id(), 1, None, "pub-0000001", &ctx())
        .await
        .expect("v1");
    f.service
        .update_draft(
            p,
            w.workflow.id(),
            1,
            br#"{"kind":"Workflow","v":2}"#,
            &ctx(),
        )
        .await
        .expect("edit");
    f.service
        .publish(p, w.workflow.id(), 2, None, "pub-0000002", &ctx())
        .await
        .expect("v2");
    f.service
        .update_draft(
            p,
            w.workflow.id(),
            2,
            br#"{"kind":"Workflow","v":3}"#,
            &ctx(),
        )
        .await
        .expect("edit again");
    w.workflow.id()
}

#[tokio::test]
async fn by_default_the_active_version_is_compared_with_the_draft() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let id = published_twice(&f, &p).await;
    let d = f
        .service
        .diff(&p, id, None, None, &ctx())
        .await
        .expect("diff");
    assert_eq!((d.from, d.to), (DiffSide::Version(2), DiffSide::Draft(3)));
    // The documents handed to the adapter are the stored ones.
    assert_eq!(d.diff.nodes_added, [r#"{"kind":"Workflow","v":2}"#]);
    assert_eq!(d.diff.nodes_removed, [r#"{"kind":"Workflow","v":3}"#]);
    assert!(!d.diff.identical);
}

#[tokio::test]
async fn any_two_versions_and_the_draft_can_be_compared() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let id = published_twice(&f, &p).await;
    let d = f
        .service
        .diff(
            &p,
            id,
            Some(DiffRef::Version(1)),
            Some(DiffRef::Version(2)),
            &ctx(),
        )
        .await
        .expect("diff");
    assert_eq!((d.from, d.to), (DiffSide::Version(1), DiffSide::Version(2)));
    let same = f
        .service
        .diff(&p, id, Some(DiffRef::Draft), Some(DiffRef::Draft), &ctx())
        .await
        .expect("diff");
    assert!(same.diff.identical);
    let missing = f
        .service
        .diff(&p, id, Some(DiffRef::Version(9)), None, &ctx())
        .await;
    assert!(matches!(missing, Err(AppError::NotFound)), "{missing:?}");
}

#[tokio::test]
async fn a_workflow_with_no_version_compares_nothing_with_its_draft() {
    let f = fixture();
    let p = principal(f.tenant, Role::Admin);
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let d = f
        .service
        .diff(&p, w.workflow.id(), None, None, &ctx())
        .await
        .expect("diff");
    assert_eq!((d.from, d.to), (DiffSide::Empty, DiffSide::Draft(1)));
}

#[tokio::test]
async fn diffing_needs_definition_read_and_a_workflow_of_the_callers_tenant() {
    let f = fixture();
    let w = create(&f, Role::Admin, "key-0000001")
        .await
        .expect("created");
    let stranger = principal(TenantId::generate(), Role::Admin);
    let result = f
        .service
        .diff(&stranger, w.workflow.id(), None, None, &ctx())
        .await;
    assert!(matches!(result, Err(AppError::NotFound)), "{result:?}");
    // Reading is enough to compare (a viewer may), writing is not needed.
    let viewer = principal(f.tenant, Role::Viewer);
    assert!(
        f.service
            .diff(&viewer, w.workflow.id(), None, None, &ctx())
            .await
            .is_ok()
    );
}

// ------------------------------------------------------------------------- a hostile document

/// A document can pin as many distinct artifacts as it has nodes (ten thousand). Naming the ones
/// that were never built must cost one pass over the nodes, not one pass per artifact.
#[test]
fn eight_thousand_unknown_artifacts_are_found_in_one_pass() {
    use sagat_application::definitions::unknown_script_problems;
    use sagat_domain::scripts::{Digest as ScriptDigest, activity_of};

    const N: usize = 8_000;
    let digests: Vec<ScriptDigest> = (0..N)
        .map(|i| ScriptDigest::parse(&format!("sha256:{i:064x}")).expect("a digest"))
        .collect();
    let nodes: Vec<Node> = digests
        .iter()
        .enumerate()
        .map(|(i, digest)| {
            let node = Node::new(
                NodeId::parse(&format!("n{i}")).expect("id"),
                NodeKind::Activity {
                    activity: ActivityName::parse(&activity_of(digest)).expect("name"),
                    retry: RetryPolicy::DEFAULT,
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            );
            if i + 1 < N {
                node.with_next(NodeId::parse(&format!("n{}", i + 1)).expect("id"))
            } else {
                node
            }
        })
        .collect();
    let definition = WorkflowDefinition::new(
        Slug::parse("many-scripts").expect("slug"),
        NodeId::parse("n0").expect("id"),
        nodes,
    )
    .expect("valid");
    let started = std::time::Instant::now();
    let problems = unknown_script_problems(&definition, &digests);
    let took = started.elapsed();
    assert_eq!(problems.len(), N);
    assert!(problems.iter().all(|p| p.code == "UNKNOWN_SCRIPT"));
    assert!(
        took < std::time::Duration::from_secs(1),
        "naming {N} unknown artifacts took {took:?}"
    );
    // What was built is not reported.
    assert!(unknown_script_problems(&definition, &digests[..10]).len() == 10);
}

//! Use-case tests for workflow definitions, against an in-memory store (T002-005).
//!
//! The fake mirrors the contract of `DefinitionStore` (tenant filter, name uniqueness, revision
//! compare-and-swap, idempotency replay). It proves the *service* logic: authorization before
//! storage, tenant derivation, audit records, error mapping. SQL-level guarantees are proven
//! against a real PostgreSQL in `tests/integration/002-durable-execution`.
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
    VersionRecord, WorkflowRecord,
};
use sagat_application::error::{AppError, ConflictKind, StoreError};
use sagat_application::identity::{PageRequest, RequestContext};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Permission, Principal, Role, Slug, Subject, TenantId,
    WorkspaceId,
};
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
}

#[derive(Clone, Default)]
struct FakeStore(Arc<Mutex<State>>);

impl FakeStore {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.lock().expect("lock"))
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

    async fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        _release_notes: Option<&str>,
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
                return Ok(Some(Applied::Done((existing.clone(), false))));
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

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        self.outcome.clone()?;
        let node = Node::new(
            NodeId::parse("a").expect("id"),
            NodeKind::Activity {
                activity: ActivityName::parse("act.a").expect("name"),
                retry: RetryPolicy::DEFAULT,
                compensate_with: None,
                reconcile_with: None,
                idempotency_template: None,
            },
        );
        let definition = WorkflowDefinition::new(
            Slug::parse(self.name).expect("slug"),
            NodeId::parse("a").expect("id"),
            vec![node],
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
    })
}

fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some("0af7651916cd43dd8448eb211c80319c".into()),
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

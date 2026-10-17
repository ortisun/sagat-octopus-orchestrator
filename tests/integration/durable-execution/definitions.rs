//! `PgDefinitionStore`: atomicity, compare-and-swap, numbering, idempotency, immutability and
//! tenant isolation, all enforced by SQL rather than by the application (FR-001-002, FR-002-008).

use std::collections::BTreeSet;

use sagat_application::definitions::{Applied, DefinitionStore, Idempotency};
use sagat_application::error::{ConflictKind, StoreError};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Principal, Workspace, WorkspaceId,
};
use sagat_domain::workflow::Digest;
use sagat_infra_postgres::{PgDefinitionStore, PgIdentityStore};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::support::{
    RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, TestDb, db_error, migrated_db, now, slug, two_tenants,
};
use sagat_application::identity::IdentityStore;

struct World {
    db: TestDb,
    store: PgDefinitionStore,
    a: Principal,
    b: Principal,
    ns_a: Namespace,
    ns_b: Namespace,
}

async fn namespace(identity: &PgIdentityStore, p: &Principal, name: &str) -> Namespace {
    let ws = Workspace::create(
        p,
        WorkspaceId::generate(),
        slug(&format!("ws-{name}")),
        now(),
    )
    .expect("workspace");
    identity
        .create_workspace(
            &ws,
            &AuditRecord::by(
                p,
                AuditAction::WorkspaceCreated,
                ResourceType::Workspace,
                ws.id().to_string(),
                None,
                now(),
            ),
        )
        .await
        .expect("persist workspace");
    let ns = Namespace::create(
        p,
        &ws,
        NamespaceId::generate(),
        slug(name),
        Environment::Dev,
        now(),
    )
    .expect("namespace");
    identity
        .create_namespace(
            &ns,
            &AuditRecord::by(
                p,
                AuditAction::NamespaceCreated,
                ResourceType::Namespace,
                ns.id().to_string(),
                None,
                now(),
            ),
        )
        .await
        .expect("persist namespace");
    ns
}

async fn world() -> World {
    let db = migrated_db().await;
    let t = two_tenants(&db).await;
    let ns_a = namespace(&t.store, &t.admin_a, "payments").await;
    let ns_b = namespace(&t.store, &t.admin_b, "payments").await;
    World {
        store: PgDefinitionStore::new(db.pool.clone()),
        a: t.admin_a,
        b: t.admin_b,
        ns_a,
        ns_b,
        db,
    }
}

fn idem(scope: &str, key: &str, fingerprint: &str) -> Idempotency {
    Idempotency {
        scope: scope.into(),
        key: key.into(),
        fingerprint: fingerprint.as_bytes().to_vec(),
    }
}

fn audit(p: &Principal, action: AuditAction, rt: ResourceType, id: &str) -> AuditRecord {
    AuditRecord::by(p, action, rt, id, None, now())
}

const DRAFT: &[u8] = br#"{"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow"}"#;

async fn create(
    w: &World,
    p: &Principal,
    ns: &Namespace,
    name: &str,
    key: &str,
) -> Result<Workflow, StoreError> {
    let workflow =
        Workflow::create(p, ns, WorkflowId::generate(), slug(name), now()).expect("domain");
    let a = audit(
        p,
        AuditAction::WorkflowCreated,
        ResourceType::Workflow,
        &workflow.id().to_string(),
    );
    match w
        .store
        .create_workflow(
            &workflow,
            DRAFT,
            &a,
            &idem(&format!("workflow.create:{}", ns.id()), key, name),
            sagat_domain::quota::MAX_LIMIT,
        )
        .await?
    {
        Applied::Done(()) => Ok(workflow),
        Applied::Replayed(id) => panic!("unexpected replay of {id}"),
    }
}

fn digest(byte: u8) -> Digest {
    Digest::new([byte; 32])
}

fn no_database_definition() -> sagat_domain::workflow::WorkflowDefinition {
    use sagat_domain::workflow::{Node, NodeId, NodeKind, WorkflowDefinition};
    let end = NodeId::parse("end").expect("id");
    WorkflowDefinition::new(
        slug("invoice-flow"),
        end.clone(),
        vec![Node::new(end, NodeKind::End { output: None })],
    )
    .expect("definition")
}

async fn publish(
    w: &World,
    p: &Principal,
    id: WorkflowId,
    revision: u64,
    byte: u8,
    key: &str,
) -> Result<Option<Applied<(sagat_domain::definition::WorkflowVersion, bool)>>, StoreError> {
    let version = WorkflowVersionId::generate();
    w.store
        .publish(
            p,
            id,
            revision,
            version,
            digest(byte),
            format!("{{\"v\":{byte}}}").as_bytes(),
            &no_database_definition(),
            Some("notes"),
            &[],
            &[],
            &sagat_application::definitions::PublishContext::default(),
            &audit(
                p,
                AuditAction::WorkflowPublished,
                ResourceType::WorkflowVersion,
                &version.to_string(),
            ),
            &idem(
                &format!("workflow.publish:{id}"),
                key,
                &format!("{revision}:{byte}"),
            ),
            now(),
        )
        .await
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .expect("count")
}

// ------------------------------------------------------------------ creating

#[tokio::test]
async fn a_workflow_is_stored_with_its_draft_and_its_audit_record_together() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let record = w
        .store
        .get_workflow(w.a.tenant_id(), wf.id())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(record.workflow, wf);
    assert_eq!(record.workflow.draft_revision(), 1);
    assert!(record.active.is_none());
    let stored: serde_json::Value = serde_json::from_slice(&record.draft).expect("json");
    assert_eq!(stored["kind"], "Workflow");
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.created'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_name_is_unique_per_namespace_and_free_in_other_namespaces_and_tenants() {
    let w = world().await;
    create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("first");
    let again = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000002").await;
    assert_eq!(
        again.map(|_| ()),
        Err(StoreError::Conflict(ConflictKind::WorkflowName))
    );
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.created'"
        )
        .await,
        1,
        "the failed create left no audit record"
    );
    assert_eq!(
        count(&w.db.pool, "SELECT count(*) FROM command_dedupe").await,
        1,
        "and no idempotency record"
    );

    let other_ns = namespace(&PgIdentityStore::new(w.db.pool.clone()), &w.a, "billing").await;
    create(&w, &w.a, &other_ns, "invoice-flow", "key-0000003")
        .await
        .expect("same name, other namespace");
    create(&w, &w.b, &w.ns_b, "invoice-flow", "key-0000001")
        .await
        .expect("same name and key, other tenant");
}

#[tokio::test]
async fn the_database_itself_refuses_a_workflow_in_another_tenants_namespace() {
    let w = world().await;
    // Bypass the application: a bug that mixed up the tenant must be stopped by the schema.
    let result = sqlx::query(
        "INSERT INTO workflows (id, tenant_id, namespace_id, name, draft, created_at, updated_at) \
         VALUES (gen_random_uuid(), $1, $2, 'sneaky-flow', '{}'::jsonb, now(), now())",
    )
    .bind(w.a.tenant_id().as_uuid())
    .bind(w.ns_b.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("composite foreign key");
    let (code, constraint) = db_error(result);
    assert_eq!(code, "23503");
    assert_eq!(
        constraint.as_deref(),
        Some("workflows_namespace_same_tenant_fk")
    );
}

#[tokio::test]
async fn a_draft_must_be_a_json_object_in_the_database_too() {
    let w = world().await;
    let result = sqlx::query(
        "INSERT INTO workflows (id, tenant_id, namespace_id, name, draft, created_at, updated_at) \
         VALUES (gen_random_uuid(), $1, $2, 'array-flow', '[1]'::jsonb, now(), now())",
    )
    .bind(w.a.tenant_id().as_uuid())
    .bind(w.ns_a.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("check constraint");
    assert_eq!(
        db_error(result),
        (
            "23514".to_owned(),
            Some("workflows_draft_is_object".to_owned())
        )
    );
}

// ------------------------------------------------------------------ idempotency

#[tokio::test]
async fn a_repeated_create_with_the_same_key_replays_and_changes_nothing() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("first");
    let second = Workflow::create(
        &w.a,
        &w.ns_a,
        WorkflowId::generate(),
        slug("invoice-flow"),
        now(),
    )
    .expect("domain");
    let outcome = w
        .store
        .create_workflow(
            &second,
            DRAFT,
            &audit(
                &w.a,
                AuditAction::WorkflowCreated,
                ResourceType::Workflow,
                &second.id().to_string(),
            ),
            &idem(
                &format!("workflow.create:{}", w.ns_a.id()),
                "key-0000001",
                "invoice-flow",
            ),
            sagat_domain::quota::MAX_LIMIT,
        )
        .await
        .expect("replay");
    assert_eq!(outcome, Applied::Replayed(wf.id().to_string()));
    assert_eq!(count(&w.db.pool, "SELECT count(*) FROM workflows").await, 1);
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.created'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn the_same_key_for_a_different_request_is_refused() {
    let w = world().await;
    create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("first");
    let other = Workflow::create(
        &w.a,
        &w.ns_a,
        WorkflowId::generate(),
        slug("other-flow"),
        now(),
    )
    .expect("domain");
    let result = w
        .store
        .create_workflow(
            &other,
            DRAFT,
            &audit(
                &w.a,
                AuditAction::WorkflowCreated,
                ResourceType::Workflow,
                &other.id().to_string(),
            ),
            &idem(
                &format!("workflow.create:{}", w.ns_a.id()),
                "key-0000001",
                "different fingerprint",
            ),
            sagat_domain::quota::MAX_LIMIT,
        )
        .await;
    assert_eq!(result, Err(StoreError::IdempotencyKeyReused));
    assert_eq!(count(&w.db.pool, "SELECT count(*) FROM workflows").await, 1);
}

#[tokio::test]
async fn the_same_key_in_another_tenant_or_scope_is_independent() {
    let w = world().await;
    create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("tenant a");
    create(&w, &w.b, &w.ns_b, "invoice-flow", "key-0000001")
        .await
        .expect("tenant b, same key, same name");
    create(&w, &w.a, &w.ns_a, "invoice-other", "key-0000001")
        .await
        .expect_err("same tenant and scope: this key is taken by a different request");
}

#[tokio::test]
async fn sixteen_simultaneous_creates_with_one_key_produce_one_workflow() {
    let w = world().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let store = w.store.clone();
        let p = w.a.clone();
        let ns = w.ns_a.clone();
        tasks.push(tokio::spawn(async move {
            let wf = Workflow::create(&p, &ns, WorkflowId::generate(), slug("invoice-flow"), now())
                .expect("domain");
            let a = audit(
                &p,
                AuditAction::WorkflowCreated,
                ResourceType::Workflow,
                &wf.id().to_string(),
            );
            store
                .create_workflow(
                    &wf,
                    DRAFT,
                    &a,
                    &idem(
                        &format!("workflow.create:{}", ns.id()),
                        "key-0000001",
                        "invoice-flow",
                    ),
                    sagat_domain::quota::MAX_LIMIT,
                )
                .await
        }));
    }
    let mut done = 0;
    let mut replayed = BTreeSet::new();
    for task in tasks {
        match task.await.expect("join") {
            Ok(Applied::Done(())) => done += 1,
            Ok(Applied::Replayed(id)) => {
                replayed.insert(id);
            }
            // A racer that lost on the name uniqueness before the key was visible is also fine:
            // what matters is that nothing was created twice.
            Err(StoreError::Conflict(ConflictKind::WorkflowName)) => {}
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!(done, 1, "exactly one request performs the write");
    assert_eq!(count(&w.db.pool, "SELECT count(*) FROM workflows").await, 1);
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.created'"
        )
        .await,
        1
    );
    assert!(replayed.len() <= 1, "every replay names the same workflow");
}

// ------------------------------------------------------------------ editing: compare-and-swap

#[tokio::test]
async fn an_edit_bumps_the_revision_and_audits_atomically() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let a = audit(
        &w.a,
        AuditAction::WorkflowDraftUpdated,
        ResourceType::Workflow,
        &wf.id().to_string(),
    );
    let updated = w
        .store
        .update_draft(w.a.tenant_id(), wf.id(), 1, br#"{"v":2}"#, &a, now())
        .await
        .expect("update")
        .expect("exists");
    assert_eq!(updated.workflow.draft_revision(), 2);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&updated.draft).expect("json"),
        serde_json::json!({"v": 2})
    );
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.draft_updated'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn eight_editors_racing_on_one_revision_have_exactly_one_winner() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let mut tasks = Vec::new();
    for i in 0..8 {
        let store = w.store.clone();
        let p = w.a.clone();
        let id = wf.id();
        tasks.push(tokio::spawn(async move {
            let a = audit(
                &p,
                AuditAction::WorkflowDraftUpdated,
                ResourceType::Workflow,
                &id.to_string(),
            );
            store
                .update_draft(
                    p.tenant_id(),
                    id,
                    1,
                    format!("{{\"editor\":{i}}}").as_bytes(),
                    &a,
                    now(),
                )
                .await
        }));
    }
    let (mut won, mut lost) = (0, 0);
    for task in tasks {
        match task.await.expect("join") {
            Ok(Some(_)) => won += 1,
            Err(StoreError::RevisionConflict { current: 2 }) => lost += 1,
            other => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!((won, lost), (1, 7));
    let revision: i64 = sqlx::query_scalar("SELECT draft_revision FROM workflows")
        .fetch_one(&w.db.pool)
        .await
        .expect("revision");
    assert_eq!(revision, 2);
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.draft_updated'"
        )
        .await,
        1,
        "only the winner is audited"
    );
}

#[tokio::test]
async fn editing_an_unknown_or_foreign_workflow_finds_nothing() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let a = audit(
        &w.b,
        AuditAction::WorkflowDraftUpdated,
        ResourceType::Workflow,
        &wf.id().to_string(),
    );
    assert!(
        w.store
            .update_draft(w.b.tenant_id(), wf.id(), 1, DRAFT, &a, now())
            .await
            .expect("ok")
            .is_none()
    );
    assert!(
        w.store
            .update_draft(w.a.tenant_id(), WorkflowId::generate(), 1, DRAFT, &a, now())
            .await
            .expect("ok")
            .is_none()
    );
    let untouched: i64 = sqlx::query_scalar("SELECT draft_revision FROM workflows")
        .fetch_one(&w.db.pool)
        .await
        .expect("revision");
    assert_eq!(untouched, 1);
}

// ------------------------------------------------------------------ publishing

#[tokio::test]
async fn publishing_numbers_versions_and_activates_the_latest() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let Some(Applied::Done((v1, created))) = publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("v1")
    else {
        panic!("not done")
    };
    assert!(created);
    assert_eq!(v1.number(), 1);
    let Some(Applied::Done((v2, _))) = publish(&w, &w.a, wf.id(), 1, 2, "pub-0000002")
        .await
        .expect("v2")
    else {
        panic!("not done")
    };
    assert_eq!(v2.number(), 2);

    let record = w
        .store
        .get_workflow(w.a.tenant_id(), wf.id())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(record.active.map(|v| v.number()), Some(2));
    let stored = w
        .store
        .get_version(w.a.tenant_id(), wf.id(), 1)
        .await
        .expect("get")
        .expect("v1");
    assert_eq!(stored.document, br#"{"v":1}"#);
    assert_eq!(stored.release_notes.as_deref(), Some("notes"));
    assert_eq!(stored.version.digest(), &digest(1));
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workflow.published'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn publishing_the_same_digest_again_reactivates_that_version_without_a_new_number() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("v1");
    publish(&w, &w.a, wf.id(), 1, 2, "pub-0000002")
        .await
        .expect("v2");
    let Some(Applied::Done((back, created))) = publish(&w, &w.a, wf.id(), 1, 1, "pub-0000003")
        .await
        .expect("again")
    else {
        panic!("not done")
    };
    assert!(!created);
    assert_eq!(back.number(), 1);
    assert_eq!(
        count(&w.db.pool, "SELECT count(*) FROM workflow_versions").await,
        2
    );
    let record = w
        .store
        .get_workflow(w.a.tenant_id(), wf.id())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(
        record.active.map(|v| v.number()),
        Some(1),
        "rolled back to version 1"
    );
}

#[tokio::test]
async fn ten_simultaneous_publishers_get_gapless_distinct_numbers() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let mut tasks = Vec::new();
    for i in 0..10u8 {
        let store = w.store.clone();
        let p = w.a.clone();
        let id = wf.id();
        tasks.push(tokio::spawn(async move {
            let version = WorkflowVersionId::generate();
            store
                .publish(
                    &p,
                    id,
                    1,
                    version,
                    digest(i + 1),
                    b"{}",
                    &no_database_definition(),
                    None,
                    &[],
                    &[],
                    &sagat_application::definitions::PublishContext::default(),
                    &audit(
                        &p,
                        AuditAction::WorkflowPublished,
                        ResourceType::WorkflowVersion,
                        &version.to_string(),
                    ),
                    &idem(
                        &format!("workflow.publish:{id}"),
                        &format!("pub-000000{i}"),
                        &i.to_string(),
                    ),
                    now(),
                )
                .await
        }));
    }
    let mut numbers = BTreeSet::new();
    for task in tasks {
        let Some(Applied::Done((v, true))) = task.await.expect("join").expect("publish") else {
            panic!("not a fresh version")
        };
        numbers.insert(v.number());
    }
    assert_eq!(
        numbers,
        (1..=10).collect::<BTreeSet<u32>>(),
        "no number is skipped or reused"
    );
}

#[tokio::test]
async fn a_stale_draft_revision_cannot_be_published() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    let a = audit(
        &w.a,
        AuditAction::WorkflowDraftUpdated,
        ResourceType::Workflow,
        &wf.id().to_string(),
    );
    w.store
        .update_draft(w.a.tenant_id(), wf.id(), 1, DRAFT, &a, now())
        .await
        .expect("edit");
    let result = publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001").await;
    assert_eq!(
        result.map(|_| ()),
        Err(StoreError::RevisionConflict { current: 2 })
    );
    assert_eq!(
        count(&w.db.pool, "SELECT count(*) FROM workflow_versions").await,
        0
    );
    assert_eq!(
        count(
            &w.db.pool,
            "SELECT count(*) FROM command_dedupe WHERE scope LIKE 'workflow.publish:%'"
        )
        .await,
        0,
        "a failed publish leaves no idempotency record"
    );
}

#[tokio::test]
async fn a_retried_publish_replays_and_a_different_one_under_the_same_key_is_refused() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("first");
    let replay = publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("replay");
    assert_eq!(replay, Some(Applied::Replayed("1".into())));
    assert_eq!(
        count(&w.db.pool, "SELECT count(*) FROM workflow_versions").await,
        1
    );
    let reused = publish(&w, &w.a, wf.id(), 1, 2, "pub-0000001").await;
    assert_eq!(reused.map(|_| ()), Err(StoreError::IdempotencyKeyReused));
}

#[tokio::test]
async fn publishing_a_foreign_or_unknown_workflow_finds_nothing() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    assert_eq!(
        publish(&w, &w.b, wf.id(), 1, 1, "pub-0000001")
            .await
            .expect("ok"),
        None
    );
    assert_eq!(
        publish(&w, &w.a, WorkflowId::generate(), 1, 1, "pub-0000002")
            .await
            .expect("ok"),
        None
    );
    assert_eq!(
        count(&w.db.pool, "SELECT count(*) FROM workflow_versions").await,
        0
    );
}

// ------------------------------------------------------------------ immutability

#[tokio::test]
async fn a_published_version_cannot_be_changed_even_by_the_owner_in_replica_mode() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("published");

    let mut conn = w.db.pool.acquire().await.expect("connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("replica mode");
    for (statement, sqlstate) in [
        ("UPDATE workflow_versions SET document = '\\x7b7d'", "23001"),
        (
            "UPDATE workflow_versions SET digest = repeat('0', 64)",
            "23001",
        ),
        ("DELETE FROM workflow_versions", "23001"),
        // Plain TRUNCATE is stopped by PostgreSQL itself (the table is referenced by `workflows`);
        // CASCADE gets past that check and meets the trigger.
        ("TRUNCATE workflow_versions", "0A000"),
        ("TRUNCATE workflow_versions CASCADE", "23001"),
    ] {
        let error = sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .expect_err(statement);
        assert_eq!(db_error(error).0, sqlstate, "{statement}");
    }
    let intact: Vec<u8> = sqlx::query_scalar("SELECT document FROM workflow_versions")
        .fetch_one(&mut *conn)
        .await
        .expect("document");
    assert_eq!(intact, br#"{"v":1}"#);
}

// ------------------------------------------------------------------ reading and isolation

#[tokio::test]
async fn reads_are_scoped_to_the_tenant_and_the_namespace() {
    let w = world().await;
    let mine = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("mine");
    let theirs = create(&w, &w.b, &w.ns_b, "invoice-flow", "key-0000001")
        .await
        .expect("theirs");

    assert!(
        w.store
            .get_workflow(w.a.tenant_id(), theirs.id())
            .await
            .expect("ok")
            .is_none()
    );
    assert!(
        w.store
            .get_workflow(w.b.tenant_id(), mine.id())
            .await
            .expect("ok")
            .is_none()
    );
    assert!(
        w.store
            .workflow_exists_outside(w.a.tenant_id(), theirs.id())
            .await
            .expect("ok")
    );
    assert!(
        !w.store
            .workflow_exists_outside(w.a.tenant_id(), mine.id())
            .await
            .expect("ok")
    );
    assert!(
        !w.store
            .workflow_exists_outside(w.a.tenant_id(), WorkflowId::generate())
            .await
            .expect("ok")
    );

    let listed = w
        .store
        .list_workflows(w.a.tenant_id(), w.ns_a.id(), None, 50)
        .await
        .expect("list");
    assert_eq!(
        listed.iter().map(|r| r.workflow.id()).collect::<Vec<_>>(),
        [mine.id()]
    );
    // A namespace id from the other tenant lists nothing: the tenant filter applies to the join too.
    assert!(
        w.store
            .list_workflows(w.a.tenant_id(), w.ns_b.id(), None, 50)
            .await
            .expect("list")
            .is_empty()
    );

    publish(&w, &w.a, mine.id(), 1, 1, "pub-0000001")
        .await
        .expect("published");
    assert!(
        w.store
            .get_version(w.b.tenant_id(), mine.id(), 1)
            .await
            .expect("ok")
            .is_none()
    );
    assert!(
        w.store
            .list_versions(w.b.tenant_id(), mine.id(), None, 50)
            .await
            .expect("ok")
            .is_empty()
    );
}

#[tokio::test]
async fn listing_pages_in_id_order_without_gaps_or_repeats() {
    let w = world().await;
    let mut ids = Vec::new();
    for i in 0..7 {
        ids.push(
            create(
                &w,
                &w.a,
                &w.ns_a,
                &format!("flow-{i}"),
                &format!("key-000000{i}"),
            )
            .await
            .expect("created")
            .id(),
        );
    }
    ids.sort();
    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = w
            .store
            .list_workflows(w.a.tenant_id(), w.ns_a.id(), after, 3)
            .await
            .expect("page");
        if page.is_empty() {
            break;
        }
        after = page.last().map(|r| r.workflow.id());
        seen.extend(page.iter().map(|r| r.workflow.id()));
    }
    assert_eq!(seen, ids);
}

#[tokio::test]
async fn a_namespace_is_found_by_slug_inside_the_tenant_only() {
    let w = world().await;
    let found = w
        .store
        .find_namespace_by_slug(w.a.tenant_id(), &slug("payments"))
        .await
        .expect("ok")
        .expect("exists");
    assert_eq!(
        found.id(),
        w.ns_a.id(),
        "the same slug in the other tenant is a different namespace"
    );
    assert!(
        w.store
            .find_namespace_by_slug(w.a.tenant_id(), &slug("nowhere"))
            .await
            .expect("ok")
            .is_none()
    );
}

// ------------------------------------------------------------------ roles and the runtime role

#[tokio::test]
async fn the_worker_role_is_a_valid_membership_and_unknown_roles_are_not() {
    let w = world().await;
    for (role, ok) in [("worker", true), ("root", false)] {
        let result = sqlx::query("INSERT INTO memberships (tenant_id, subject, role, created_at) VALUES ($1, 'machine-1', $2, now())")
            .bind(w.a.tenant_id().as_uuid())
            .bind(role)
            .execute(&w.db.pool)
            .await;
        assert_eq!(result.is_ok(), ok, "{role}");
        if ok {
            sqlx::query("DELETE FROM memberships WHERE subject = 'machine-1'")
                .execute(&w.db.pool)
                .await
                .expect("cleanup");
        }
    }
}

#[tokio::test]
async fn the_runtime_role_can_edit_definitions_but_not_rewrite_history() {
    let w = world().await;
    let wf = create(&w, &w.a, &w.ns_a, "invoice-flow", "key-0000001")
        .await
        .expect("created");
    publish(&w, &w.a, wf.id(), 1, 1, "pub-0000001")
        .await
        .expect("published");

    let app = PgPoolOptions::new()
        .max_connections(2)
        .connect(&w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD))
        .await
        .expect("runtime pool");
    // Allowed: read workflows and versions, edit the draft, add rows, purge idempotency records.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM workflow_versions")
        .fetch_one(&app)
        .await
        .expect("read");
    assert_eq!(n, 1);
    sqlx::query("UPDATE workflows SET draft_revision = draft_revision + 1")
        .execute(&app)
        .await
        .expect("edit");
    sqlx::query("DELETE FROM command_dedupe WHERE created_at < now() - interval '1 day'")
        .execute(&app)
        .await
        .expect("purge");
    // Refused: delete a workflow, touch a version, truncate anything.
    for statement in [
        "DELETE FROM workflows",
        "UPDATE workflow_versions SET release_notes = 'x'",
        "DELETE FROM workflow_versions",
        "TRUNCATE workflows CASCADE",
        "TRUNCATE command_dedupe",
    ] {
        let error = sqlx::query(statement)
            .execute(&app)
            .await
            .expect_err(statement);
        assert_eq!(db_error(error).0, "42501", "{statement}");
    }
}

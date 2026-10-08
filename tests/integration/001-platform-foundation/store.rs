//! `PgIdentityStore` behaviour: tenant isolation in SQL, atomicity, and failure mapping.

use std::time::Duration;

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::identity::IdentityStore;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Principal, Workspace, WorkspaceId,
};
use sagat_infra_postgres::{PgIdentityStore, PoolConfig, connect};

use crate::support::{migrated_db, now, slug, subject, two_tenants};

fn audit(
    p: &Principal,
    action: AuditAction,
    rt: ResourceType,
    id: &str,
    trace: Option<&str>,
) -> AuditRecord {
    AuditRecord::by(p, action, rt, id, trace.map(str::to_owned), now())
}

async fn add_workspace(store: &PgIdentityStore, p: &Principal, name: &str) -> Workspace {
    let ws = Workspace::create(p, WorkspaceId::generate(), slug(name), now()).expect("domain");
    let a = audit(
        p,
        AuditAction::WorkspaceCreated,
        ResourceType::Workspace,
        &ws.id().to_string(),
        None,
    );
    store
        .create_workspace(&ws, &a)
        .await
        .expect("persist workspace");
    ws
}

async fn add_namespace(
    store: &PgIdentityStore,
    p: &Principal,
    ws: &Workspace,
    name: &str,
) -> Namespace {
    let ns = Namespace::create(
        p,
        ws,
        NamespaceId::generate(),
        slug(name),
        Environment::Dev,
        now(),
    )
    .expect("domain");
    let a = audit(
        p,
        AuditAction::NamespaceCreated,
        ResourceType::Namespace,
        &ns.id().to_string(),
        None,
    );
    store
        .create_namespace(&ns, &a)
        .await
        .expect("persist namespace");
    ns
}

#[tokio::test]
async fn workspace_round_trips_with_its_tenant() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;

    let ws = add_workspace(&w.store, &w.admin_a, "platform").await;
    let loaded = w
        .store
        .get_workspace(w.tenant_a.id(), ws.id())
        .await
        .expect("query");

    assert_eq!(loaded, Some(ws));
}

#[tokio::test]
async fn reads_never_cross_the_tenant_boundary() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws_a = add_workspace(&w.store, &w.admin_a, "platform").await;
    let ns_a = add_namespace(&w.store, &w.admin_a, &ws_a, "dev").await;

    // Tenant B asks for tenant A's ids directly.
    assert_eq!(
        w.store
            .get_workspace(w.tenant_b.id(), ws_a.id())
            .await
            .expect("q"),
        None
    );
    assert_eq!(
        w.store
            .get_namespace(w.tenant_b.id(), ns_a.id())
            .await
            .expect("q"),
        None
    );
    assert!(
        w.store
            .list_workspaces(w.tenant_b.id(), None, 50)
            .await
            .expect("q")
            .is_empty()
    );
    assert!(
        w.store
            .list_namespaces(w.tenant_b.id(), None, 50)
            .await
            .expect("q")
            .is_empty()
    );

    // ... and A still sees its own.
    assert_eq!(
        w.store
            .list_workspaces(w.tenant_a.id(), None, 50)
            .await
            .expect("q"),
        vec![ws_a]
    );
    assert_eq!(
        w.store
            .list_namespaces(w.tenant_a.id(), None, 50)
            .await
            .expect("q"),
        vec![ns_a]
    );
}

#[tokio::test]
async fn memberships_are_looked_up_inside_their_own_tenant_only() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;

    let own = w
        .store
        .find_membership(w.tenant_a.id(), &subject("alice"))
        .await
        .expect("q");
    assert_eq!(own.expect("alice is in acme").role().as_str(), "admin");
    // alice is not a member of globex, and bob is not a member of acme.
    assert_eq!(
        w.store
            .find_membership(w.tenant_b.id(), &subject("alice"))
            .await
            .expect("q"),
        None
    );
    assert_eq!(
        w.store
            .find_membership(w.tenant_a.id(), &subject("bob"))
            .await
            .expect("q"),
        None
    );
}

#[tokio::test]
async fn tenants_resolve_by_slug() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;

    assert_eq!(
        w.store.find_tenant_by_slug(&slug("acme")).await.expect("q"),
        Some(w.tenant_a.clone())
    );
    assert_eq!(
        w.store
            .find_tenant_by_slug(&slug("nobody"))
            .await
            .expect("q"),
        None
    );
}

#[tokio::test]
async fn exists_outside_reports_only_other_tenants() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws_a = add_workspace(&w.store, &w.admin_a, "platform").await;
    let ns_a = add_namespace(&w.store, &w.admin_a, &ws_a, "dev").await;

    // From B's point of view A's resources exist "outside"; from A's they do not.
    assert!(
        w.store
            .workspace_exists_outside(w.tenant_b.id(), ws_a.id())
            .await
            .expect("q")
    );
    assert!(
        !w.store
            .workspace_exists_outside(w.tenant_a.id(), ws_a.id())
            .await
            .expect("q")
    );
    assert!(
        w.store
            .namespace_exists_outside(w.tenant_b.id(), ns_a.id())
            .await
            .expect("q")
    );
    assert!(
        !w.store
            .namespace_exists_outside(w.tenant_a.id(), ns_a.id())
            .await
            .expect("q")
    );
    // An id that exists nowhere is not "outside" either.
    assert!(
        !w.store
            .workspace_exists_outside(w.tenant_b.id(), WorkspaceId::generate())
            .await
            .expect("q")
    );
}

#[tokio::test]
async fn duplicate_slugs_conflict_inside_a_tenant_and_are_free_across_tenants() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws_a = add_workspace(&w.store, &w.admin_a, "platform").await;

    let dup = Workspace::create(&w.admin_a, WorkspaceId::generate(), slug("platform"), now())
        .expect("domain");
    let a = audit(
        &w.admin_a,
        AuditAction::WorkspaceCreated,
        ResourceType::Workspace,
        "x",
        None,
    );
    assert_eq!(
        w.store.create_workspace(&dup, &a).await,
        Err(StoreError::Conflict(ConflictKind::WorkspaceSlug))
    );

    add_workspace(&w.store, &w.admin_b, "platform").await; // other tenant: fine

    add_namespace(&w.store, &w.admin_a, &ws_a, "dev").await;
    let ns_dup = Namespace::create(
        &w.admin_a,
        &ws_a,
        NamespaceId::generate(),
        slug("dev"),
        Environment::Prod,
        now(),
    )
    .expect("domain");
    let a = audit(
        &w.admin_a,
        AuditAction::NamespaceCreated,
        ResourceType::Namespace,
        "x",
        None,
    );
    assert_eq!(
        w.store.create_namespace(&ns_dup, &a).await,
        Err(StoreError::Conflict(ConflictKind::NamespaceSlug))
    );
}

#[tokio::test]
async fn a_failed_audit_insert_rolls_the_state_change_back() {
    // State change and audit record are one transaction: if the audit row is rejected, the
    // workspace must not remain (AGENTS.md rule 3).
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws = Workspace::create(&w.admin_a, WorkspaceId::generate(), slug("platform"), now())
        .expect("domain");
    let bad_audit = audit(
        &w.admin_a,
        AuditAction::WorkspaceCreated,
        ResourceType::Workspace,
        "x",
        Some("not-a-trace-id"),
    );

    let result = w.store.create_workspace(&ws, &bad_audit).await;

    assert!(
        matches!(result, Err(StoreError::Internal { .. })),
        "{result:?}"
    );
    assert_eq!(
        w.store
            .get_workspace(w.tenant_a.id(), ws.id())
            .await
            .expect("q"),
        None,
        "workspace must have been rolled back with the failed audit"
    );
}

#[tokio::test]
async fn a_namespace_naming_a_workspace_of_another_tenant_is_a_missing_reference() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws_b = add_workspace(&w.store, &w.admin_b, "platform").await;

    // Forge a namespace row for tenant A that points at tenant B's workspace (bypassing the
    // domain constructor, which would refuse), as a buggy caller of the port might.
    let forged = Namespace::restore(
        NamespaceId::generate(),
        w.tenant_a.id(),
        ws_b.id(),
        slug("dev"),
        Environment::Dev,
        now(),
    );
    let a = audit(
        &w.admin_a,
        AuditAction::NamespaceCreated,
        ResourceType::Namespace,
        "x",
        None,
    );

    assert_eq!(
        w.store.create_namespace(&forged, &a).await,
        Err(StoreError::MissingReference)
    );
    assert!(
        w.store
            .list_namespaces(w.tenant_a.id(), None, 50)
            .await
            .expect("q")
            .is_empty()
    );
}

#[tokio::test]
async fn keyset_pagination_walks_every_row_exactly_once_in_order() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let mut expected = Vec::new();
    for i in 0..7 {
        expected.push(
            add_workspace(&w.store, &w.admin_a, &format!("ws-{i}"))
                .await
                .id(),
        );
    }

    let mut seen = Vec::new();
    let mut after = None;
    for _ in 0..10 {
        let page = w
            .store
            .list_workspaces(w.tenant_a.id(), after, 3)
            .await
            .expect("page");
        if page.is_empty() {
            break;
        }
        after = page.last().map(Workspace::id);
        seen.extend(page.iter().map(Workspace::id));
    }

    assert_eq!(
        seen, expected,
        "UUIDv7 ids give creation order; no gaps, no repeats"
    );
}

// ------------------------------------------------------------------------------ fault injection

#[tokio::test]
async fn a_statement_that_exceeds_the_timeout_is_reported_as_unavailable() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;

    // A second connection holds an exclusive lock, so reads on `workspaces` block.
    let mut blocker = db.pool.acquire().await.expect("blocker connection");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE workspaces IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");

    let mut config = PoolConfig::new(db.url.clone());
    config.statement_timeout = Duration::from_millis(300);
    let pool = connect(&config)
        .await
        .expect("pool with a short statement timeout");
    let impatient = PgIdentityStore::new(pool);

    let started = std::time::Instant::now();
    // The store must give up on its own (statement timeout); this outer bound only makes a
    // regression fail fast instead of hanging the whole suite.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        impatient.get_workspace(w.tenant_a.id(), WorkspaceId::generate()),
    )
    .await
    .expect("the store hung instead of enforcing its statement timeout");

    assert!(
        matches!(result, Err(StoreError::Unavailable { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "must give up quickly, not hang"
    );

    // Releasing the lock restores service without restarting anything.
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");
    drop(blocker);
    assert_eq!(
        impatient
            .get_workspace(w.tenant_a.id(), WorkspaceId::generate())
            .await,
        Ok(None)
    );
}

#[tokio::test]
async fn the_store_recovers_after_every_connection_is_killed() {
    let db = migrated_db().await;
    let w = two_tenants(&db).await;
    let ws = add_workspace(&w.store, &w.admin_a, "platform").await;

    // Simulate a database restart / network cut: terminate all other backends.
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()")
        .execute(&db.pool)
        .await
        .expect("terminate");

    // Failures during the outage must be retryable (Unavailable), never Internal or wrong data;
    // within a few attempts the pool re-establishes connections and the data is intact.
    let mut recovered = None;
    for _ in 0..5 {
        match w.store.get_workspace(w.tenant_a.id(), ws.id()).await {
            Ok(found) => {
                recovered = Some(found);
                break;
            }
            Err(error) => assert!(matches!(error, StoreError::Unavailable { .. }), "{error:?}"),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        recovered,
        Some(Some(ws)),
        "service resumed and nothing was lost"
    );
}

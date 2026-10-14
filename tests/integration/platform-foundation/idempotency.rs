//! Creating a workspace or a namespace with an `Idempotency-Key` is safe to retry (gap G8): the
//! second request is answered with the first one's resource, against a real PostgreSQL.

use sagat_application::clock::SystemClock;
use sagat_application::error::{AppError, ConflictKind};
use sagat_application::identity::{IdentityService, RequestContext};
use sagat_infra_postgres::PgIdentityStore;

use crate::support::{migrated_db, provision};

const KEY: &str = "create-platform-0001";

fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".into()),
        span_id: None,
    }
}

async fn count(pool: &sqlx::PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn a_retried_workspace_creation_returns_the_original_and_writes_it_once() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;

    let first = svc
        .create_workspace_idempotent(&admin, "platform", KEY, &ctx())
        .await
        .expect("created");
    let again = svc
        .create_workspace_idempotent(&admin, "platform", KEY, &ctx())
        .await
        .expect("the retry is answered with the same workspace");

    assert_eq!(again, first);
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 1);
    assert_eq!(
        count(
            &db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workspace.created'"
        )
        .await,
        1,
        "a replay is not a second event"
    );
    assert_eq!(
        count(&db.pool, "SELECT count(*) FROM command_dedupe").await,
        1
    );
}

#[tokio::test]
async fn the_same_key_for_another_workspace_is_refused_and_a_plain_repeat_still_conflicts() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    svc.create_workspace_idempotent(&admin, "platform", KEY, &ctx())
        .await
        .expect("created");

    let other = svc
        .create_workspace_idempotent(&admin, "billing", KEY, &ctx())
        .await;
    assert_eq!(other, Err(AppError::IdempotencyKeyReused));

    // Without a key nothing changed: the second creation of a taken slug is a conflict.
    let plain = svc.create_workspace(&admin, "platform", &ctx()).await;
    assert_eq!(plain, Err(AppError::Conflict(ConflictKind::WorkspaceSlug)));
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 1);
}

#[tokio::test]
async fn requests_racing_with_one_key_create_exactly_one_workspace() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;

    let mut tasks = Vec::new();
    for _ in 0..12 {
        let (svc, admin) = (svc.clone(), admin.clone());
        tasks.push(tokio::spawn(async move {
            svc.create_workspace_idempotent(&admin, "platform", KEY, &ctx())
                .await
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        let workspace = task
            .await
            .expect("joined")
            .expect("every racer is answered, none is told it conflicts");
        ids.push(workspace.id());
    }

    ids.dedup();
    assert_eq!(ids.len(), 1, "all twelve got the same workspace");
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 1);
    assert_eq!(
        count(
            &db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workspace.created'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_retried_namespace_creation_returns_the_original_and_writes_it_once() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    let ws = svc
        .create_workspace(&admin, "platform", &ctx())
        .await
        .expect("workspace");

    let first = svc
        .create_namespace_idempotent(&admin, ws.id(), "dev", "dev", KEY, &ctx())
        .await
        .expect("created");
    let again = svc
        .create_namespace_idempotent(&admin, ws.id(), "dev", "dev", KEY, &ctx())
        .await
        .expect("replayed");
    let other = svc
        .create_namespace_idempotent(&admin, ws.id(), "dev", "prod", KEY, &ctx())
        .await;

    assert_eq!(again, first);
    assert_eq!(other, Err(AppError::IdempotencyKeyReused));
    assert_eq!(count(&db.pool, "SELECT count(*) FROM namespaces").await, 1);
}

#[tokio::test]
async fn the_idempotency_key_of_one_tenant_says_nothing_to_another() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    provision(&store, "globex", "bob").await;
    let alice = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    let bob = svc
        .authenticate("globex", "bob")
        .await
        .expect("session")
        .principal;

    let a = svc
        .create_workspace_idempotent(&alice, "platform", KEY, &ctx())
        .await
        .expect("acme");
    let b = svc
        .create_workspace_idempotent(&bob, "billing", KEY, &ctx())
        .await
        .expect("globex uses the same key text for its own, different request");

    assert_ne!(a.id(), b.id());
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 2);
}

#[tokio::test]
async fn concurrent_different_bodies_with_one_key_commit_only_the_winning_resource_and_audit() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(12));
    let mut tasks = Vec::new();
    for i in 0..12 {
        let (svc, admin, barrier) = (svc.clone(), admin.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            svc.create_workspace_idempotent(&admin, &format!("workspace-{i}"), KEY, &ctx())
                .await
        }));
    }
    let mut successes = 0;
    for task in tasks {
        match task.await.expect("joined") {
            Ok(_) => successes += 1,
            Err(error) => assert_eq!(error, AppError::IdempotencyKeyReused),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 1);
    assert_eq!(
        count(&db.pool, "SELECT count(*) FROM command_dedupe").await,
        1
    );
    assert_eq!(
        count(
            &db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workspace.created'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn namespace_keys_are_scoped_to_the_workspace_and_racing_environments_cannot_share_a_key() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    let a = svc
        .create_workspace(&admin, "one", &ctx())
        .await
        .expect("workspace");
    let b = svc
        .create_workspace(&admin, "two", &ctx())
        .await
        .expect("workspace");
    let context = ctx();
    let (dev, prod) = tokio::join!(
        svc.create_namespace_idempotent(&admin, a.id(), "first", "dev", KEY, &context),
        svc.create_namespace_idempotent(&admin, a.id(), "first", "prod", KEY, &context),
    );
    let (winner, loser) = if dev.is_ok() {
        (dev, prod)
    } else {
        (prod, dev)
    };
    let first = winner.expect("one creator succeeds");
    assert_eq!(loser, Err(AppError::IdempotencyKeyReused));
    let second = svc
        .create_namespace_idempotent(&admin, b.id(), "second", "prod", KEY, &ctx())
        .await
        .expect("different workspace has an independent key");
    assert_ne!(first.id(), second.id());
    assert_eq!(count(&db.pool, "SELECT count(*) FROM namespaces").await, 2);
    assert_eq!(
        count(
            &db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'namespace.created'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn failed_audit_rolls_back_the_resource_and_key_and_a_retry_can_commit() {
    let db = migrated_db().await;
    let store = PgIdentityStore::new(db.pool.clone());
    let svc = IdentityService::new(store.clone(), SystemClock);
    provision(&store, "acme", "alice").await;
    let admin = svc
        .authenticate("acme", "alice")
        .await
        .expect("session")
        .principal;
    sqlx::raw_sql("CREATE FUNCTION reject_identity_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action = 'workspace.created' THEN RAISE EXCEPTION 'injected audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_identity_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION reject_identity_audit();")
        .execute(&db.pool).await.expect("inject audit failure");
    assert!(
        svc.create_workspace_idempotent(&admin, "platform", KEY, &ctx())
            .await
            .is_err()
    );
    assert_eq!(count(&db.pool, "SELECT count(*) FROM workspaces").await, 0);
    assert_eq!(
        count(&db.pool, "SELECT count(*) FROM command_dedupe").await,
        0
    );
    sqlx::query("DROP TRIGGER reject_identity_audit ON audit_records")
        .execute(&db.pool)
        .await
        .expect("remove fault");
    let first = svc
        .create_workspace_idempotent(&admin, "platform", KEY, &ctx())
        .await
        .expect("retry succeeds");
    // A new service instance has no in-memory replay state: the committed record is authoritative.
    let restarted = IdentityService::new(store, SystemClock);
    assert_eq!(
        restarted
            .create_workspace_idempotent(&admin, "platform", KEY, &ctx())
            .await
            .expect("replay after restart"),
        first
    );
    assert_eq!(
        count(
            &db.pool,
            "SELECT count(*) FROM audit_records WHERE action = 'workspace.created'"
        )
        .await,
        1
    );
}

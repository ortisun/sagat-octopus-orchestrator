//! US01 and US04 end to end at the service level: real use cases, real PostgreSQL.

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::error::{AppError, ConflictKind};
use sagat_application::identity::{IdentityService, PageRequest, RequestContext};
use sagat_domain::identity::{Membership, Role};
use sagat_infra_postgres::PgIdentityStore;
use sqlx::PgPool;

use crate::support::{TestDb, eventually, migrated_db, now, provision, subject};

type Service = IdentityService<PgIdentityStore, SystemClock>;

const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some(TRACE.into()),
    }
}

fn service(db: &TestDb) -> (Service, PgIdentityStore) {
    let store = PgIdentityStore::new(db.pool.clone());
    (IdentityService::new(store.clone(), SystemClock), store)
}

async fn audit_rows(
    pool: &PgPool,
    tenant_slug: &str,
) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT a.action, a.actor, a.resource_id, a.trace_id FROM audit_records a \
         JOIN tenants t ON t.id = a.tenant_id WHERE t.slug = $1 ORDER BY a.created_at, a.id",
    )
    .bind(tenant_slug)
    .fetch_all(pool)
    .await
    .expect("audit rows")
}

#[tokio::test]
async fn us01_an_administrator_creates_a_workspace_and_a_dev_namespace() {
    // Given an authenticated administrator
    let db = migrated_db().await;
    let (svc, store) = service(&db);
    let (tenant, _) = provision(&store, "acme", "alice").await;
    let session = svc
        .authenticate("acme", "alice")
        .await
        .expect("authenticated");

    // When they create a workspace and a dev namespace
    let ws = svc
        .create_workspace(&session.principal, "platform", &ctx())
        .await
        .expect("workspace");
    let ns = svc
        .create_namespace(&session.principal, ws.id(), "dev", "dev", &ctx())
        .await
        .expect("namespace");

    // Then both are persisted and the scope is visible through reads
    assert_eq!(
        svc.get_workspace(&session.principal, ws.id(), &ctx()).await,
        Ok(ws.clone())
    );
    assert_eq!(
        svc.get_namespace(&session.principal, ns.id(), &ctx()).await,
        Ok(ns.clone())
    );
    let listed = svc
        .list_namespaces(&session.principal, &PageRequest::default())
        .await
        .expect("list");
    assert_eq!(listed.items, vec![ns.clone()]);
    assert_eq!(ns.workspace_id(), ws.id());

    // And every change left an audit record: the privileged provisioning of the tenant and of its
    // administrator (by the system, atomically with the change), then each mutation by alice with
    // the request's trace id
    let rows = audit_rows(&db.pool, "acme").await;
    assert_eq!(
        rows,
        vec![
            (
                "tenant.provisioned".into(),
                "system:provisioning".into(),
                tenant.id().to_string(),
                None
            ),
            (
                "membership.granted".into(),
                "system:provisioning".into(),
                "alice".into(),
                None
            ),
            (
                "workspace.created".into(),
                "alice".into(),
                ws.id().to_string(),
                Some(TRACE.into())
            ),
            (
                "namespace.created".into(),
                "alice".into(),
                ns.id().to_string(),
                Some(TRACE.into())
            ),
        ]
    );
}

#[tokio::test]
async fn us04_a_tenant_a_user_cannot_see_tenant_b_data_and_the_attempt_is_recorded() {
    // Given a user of tenant A, and a workspace and namespace of tenant B
    let db = migrated_db().await;
    let (svc, store) = service(&db);
    provision(&store, "acme", "alice").await;
    provision(&store, "globex", "bob").await;
    let alice = svc
        .authenticate("acme", "alice")
        .await
        .expect("alice")
        .principal;
    let bob = svc
        .authenticate("globex", "bob")
        .await
        .expect("bob")
        .principal;
    let ws_b = svc
        .create_workspace(&bob, "secret-ws", &ctx())
        .await
        .expect("b ws");
    let ns_b = svc
        .create_namespace(&bob, ws_b.id(), "secret-dev", "prod", &ctx())
        .await
        .expect("b ns");

    // When alice requests tenant B's resources by id
    let got_ws = svc.get_workspace(&alice, ws_b.id(), &ctx()).await;
    let got_ns = svc.get_namespace(&alice, ns_b.id(), &ctx()).await;
    let create_in_b = svc
        .create_namespace(&alice, ws_b.id(), "sneaky", "dev", &ctx())
        .await;

    // Then the API reveals nothing: indistinguishable from "does not exist"
    assert_eq!(got_ws, Err(AppError::NotFound));
    assert_eq!(got_ns, Err(AppError::NotFound));
    assert_eq!(create_in_b, Err(AppError::NotFound));
    assert!(
        svc.list_workspaces(&alice, &PageRequest::default())
            .await
            .expect("list")
            .items
            .is_empty()
    );
    assert!(
        svc.list_namespaces(&alice, &PageRequest::default())
            .await
            .expect("list")
            .items
            .is_empty()
    );

    // And the attempts are recorded in ALICE's tenant, naming only what she sent. The write is
    // off the request path, and the third attempt repeats the first one (same actor, same
    // workspace), so it is collapsed: two records, not three.
    let denials = || async {
        audit_rows(&db.pool, "acme")
            .await
            .into_iter()
            .filter(|row| row.0 == "access.denied.cross_tenant")
            .collect::<Vec<_>>()
    };
    let alice_rows = eventually("two denial records", || async {
        let rows = denials().await;
        (rows.len() >= 2).then_some(rows)
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await; // a wrongly recorded third would land by now
    let alice_rows = if alice_rows.len() == 2 {
        denials().await
    } else {
        alice_rows
    };
    assert_eq!(alice_rows.len(), 2);
    for (action, actor, _, trace) in &alice_rows {
        assert_eq!(action, "access.denied.cross_tenant");
        assert_eq!(actor, "alice");
        assert_eq!(trace.as_deref(), Some(TRACE));
    }
    // The records are written off the request path, by tasks that race: which of the two lands first is not
    // decided, and the test is about what was recorded, not about the order of the writes.
    let mut recorded: Vec<_> = alice_rows.iter().map(|r| r.2.clone()).collect();
    recorded.sort();
    let mut expected = vec![ws_b.id().to_string(), ns_b.id().to_string()];
    expected.sort();
    assert_eq!(recorded, expected);

    // Tenant B's audit trail contains only its own creations, nothing about alice's probing
    let bob_actions: Vec<_> = audit_rows(&db.pool, "globex")
        .await
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(
        bob_actions,
        vec![
            "tenant.provisioned",
            "membership.granted",
            "workspace.created",
            "namespace.created"
        ]
    );

    // No tenant B names or ids leaked into tenant A's trail
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
         WHERE t.slug = 'acme' AND (a.reason ILIKE '%globex%' OR a.reason ILIKE '%secret%' OR a.actor = 'bob')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("leak query");
    assert_eq!(leaked, 0);
}

#[tokio::test]
async fn unknown_tenant_unknown_subject_and_wrong_tenant_all_fail_identically() {
    let db = migrated_db().await;
    let (svc, store) = service(&db);
    provision(&store, "acme", "alice").await;
    provision(&store, "globex", "bob").await;

    for (tenant, who) in [
        ("nope", "alice"),
        ("acme", "stranger"),
        ("globex", "alice"),
        ("acme", "bob"),
    ] {
        assert_eq!(
            svc.authenticate(tenant, who).await,
            Err(AppError::Unauthenticated),
            "tenant={tenant} subject={who}"
        );
    }
}

#[tokio::test]
async fn roles_are_enforced_end_to_end() {
    let db = migrated_db().await;
    let (svc, store) = service(&db);
    let (tenant, admin) = provision(&store, "acme", "alice").await;
    let ws = svc
        .create_workspace(&admin, "platform", &ctx())
        .await
        .expect("ws");
    store
        .insert_membership(&Membership::restore(
            tenant.id(),
            subject("dan"),
            Role::Developer,
            now(),
        ))
        .await
        .expect("grant developer");
    let dan = svc
        .authenticate("acme", "dan")
        .await
        .expect("dan")
        .principal;

    // A developer reads but cannot create scopes
    assert_eq!(
        svc.get_workspace(&dan, ws.id(), &ctx()).await,
        Ok(ws.clone())
    );
    assert!(matches!(
        svc.create_workspace(&dan, "mine", &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        svc.create_namespace(&dan, ws.id(), "dev", "dev", &ctx())
            .await,
        Err(AppError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn racing_creators_of_the_same_workspace_produce_exactly_one_winner_and_one_audit_row() {
    let db = migrated_db().await;
    let (svc, store) = service(&db);
    let (_, admin) = provision(&store, "acme", "alice").await;
    let svc = Arc::new(svc);

    let attempts: Vec<_> = (0..8)
        .map(|_| {
            let svc = Arc::clone(&svc);
            let admin = admin.clone();
            tokio::spawn(async move { svc.create_workspace(&admin, "platform", &ctx()).await })
        })
        .collect();
    let mut winners = 0;
    let mut conflicts = 0;
    for attempt in attempts {
        match attempt.await.expect("join") {
            Ok(_) => winners += 1,
            Err(AppError::Conflict(ConflictKind::WorkspaceSlug)) => conflicts += 1,
            Err(other) => panic!("unexpected outcome: {other:?}"),
        }
    }

    assert_eq!((winners, conflicts), (1, 7));
    let rows = audit_rows(&db.pool, "acme").await;
    let created = rows.iter().filter(|r| r.0 == "workspace.created").count();
    assert_eq!(created, 1, "losers must leave no audit row behind");
    assert_eq!(
        rows.len(),
        3,
        "2 provisioning records + exactly 1 workspace.created"
    );
    let workspaces: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(workspaces, 1);
}

#[tokio::test]
async fn readiness_ping_reflects_the_database() {
    let db = migrated_db().await;
    let (svc, _) = service(&db);
    assert_eq!(svc.ping().await, Ok(()));
}

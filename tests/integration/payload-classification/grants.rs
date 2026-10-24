//! Real PostgreSQL coverage for namespace PII reveal grants (feature 016).
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::reveal_grants::{RevealGrantError, RevealGrantService};
use sagat_domain::identity::{Permission, Role};
use sagat_infra_postgres::{PgRunStore, has_effective_payload_reveal_grant};

use crate::support::engine::{World, ctx, world};

type Grants = RevealGrantService<PgRunStore, SystemClock>;

fn grants(w: &World) -> Grants {
    RevealGrantService::new(w.store.clone(), SystemClock)
}

fn is_forbidden(error: &RevealGrantError) -> bool {
    matches!(
        error,
        RevealGrantError::Application(AppError::Forbidden {
            permission: Permission::MembershipManage
        })
    )
}

async fn grant_count(w: &World, namespace: sagat_domain::identity::NamespaceId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM payload_reveal_grants WHERE tenant_id=$1 AND namespace_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(namespace.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("grant count")
}

#[tokio::test]
async fn rules_tenant_boundary_self_grant_and_membership_cascade_are_enforced() {
    let w = world().await;
    let service = grants(&w);
    let namespace = w.namespace.id();
    let scope = service
        .prepare(&w.admin, namespace, &ctx())
        .await
        .expect("admin scope");

    let self_grant = service
        .grant(&scope, w.admin.subject().as_str(), &ctx())
        .await
        .expect("an administrator may grant themselves");
    assert_eq!(self_grant.subject, *w.admin.subject());
    assert_eq!(
        service
            .grant(&scope, w.admin.subject().as_str(), &ctx())
            .await
            .expect("PUT is idempotent"),
        self_grant
    );

    assert_eq!(
        service
            .grant(&scope, "unknown-subject", &ctx())
            .await
            .expect_err("unknown member"),
        RevealGrantError::UnknownSubject
    );
    let viewer = w.member("viewer-no-internal", Role::Viewer).await;
    assert_eq!(
        service
            .grant(&scope, viewer.subject().as_str(), &ctx())
            .await
            .expect_err("viewer grant would be inert"),
        RevealGrantError::RoleCannotReveal
    );

    let foreign = service
        .prepare(&w.other_admin, namespace, &ctx())
        .await
        .expect_err("foreign namespace stays opaque");
    assert_eq!(foreign, AppError::NotFound);

    let developer = w.member("cascade-developer", Role::Developer).await;
    service
        .grant(&scope, developer.subject().as_str(), &ctx())
        .await
        .expect("eligible member");
    sqlx::query("DELETE FROM memberships WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(developer.subject().as_str())
        .execute(&w.db.pool)
        .await
        .expect("remove membership");
    let cascaded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payload_reveal_grants WHERE tenant_id=$1 AND subject=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(developer.subject().as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("cascade count");
    assert_eq!(cascaded, 0);

    let grant_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
         AND action='payload.reveal_granted' AND resource_type='payload_reveal_grant'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("grant audits");
    assert_eq!(
        grant_audits, 2,
        "idempotent PUT and refused writes add no audit"
    );
}

#[tokio::test]
async fn the_namespace_cap_is_exact_under_concurrent_grants() {
    let w = world().await;
    let namespace = w.add_namespace("reveal-cap").await.id();
    let service = Arc::new(grants(&w));
    let scope = service
        .prepare(&w.admin, namespace, &ctx())
        .await
        .expect("scope");
    let subjects: Vec<String> = (0..257).map(|index| format!("cap-{index:03}")).collect();
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) \
         SELECT $1, subject, 'developer', $2 FROM unnest($3::text[]) AS subject",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(crate::support::now())
    .bind(&subjects)
    .execute(&w.db.pool)
    .await
    .expect("seed eligible members");

    let mut attempts = tokio::task::JoinSet::new();
    for subject in subjects {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        attempts.spawn(async move { service.grant(&scope, &subject, &ctx()).await });
    }
    let mut granted = 0;
    let mut limited = 0;
    while let Some(attempt) = attempts.join_next().await {
        match attempt.expect("grant task") {
            Ok(_) => granted += 1,
            Err(RevealGrantError::GrantLimit) => limited += 1,
            Err(other) => panic!("unexpected grant result: {other:?}"),
        }
    }
    assert_eq!((granted, limited), (256, 1));
    assert_eq!(grant_count(&w, namespace).await, 256);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
         AND action='payload.reveal_granted' AND resource_id LIKE $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(format!("{namespace}/%"))
    .fetch_one(&w.db.pool)
    .await
    .expect("audit count");
    assert_eq!(audits, 256);
}

#[tokio::test]
async fn a_stale_admin_snapshot_cannot_list_grant_or_revoke() {
    let w = world().await;
    let service = grants(&w);
    let namespace = w.namespace.id();
    let developer = w.member("stale-target", Role::Developer).await;
    let scope = service
        .prepare(&w.admin, namespace, &ctx())
        .await
        .expect("scope before demotion");
    service
        .grant(&scope, developer.subject().as_str(), &ctx())
        .await
        .expect("seed grant");

    sqlx::query("UPDATE memberships SET role='developer' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(w.admin.subject().as_str())
        .execute(&w.db.pool)
        .await
        .expect("demote administrator");

    let list = service
        .list(&scope, None, 10)
        .await
        .expect_err("stale list authority");
    let grant = service
        .grant(&scope, "nobody", &ctx())
        .await
        .expect_err("stale grant authority");
    let revoke = service
        .revoke(&scope, developer.subject().as_str(), &ctx())
        .await
        .expect_err("stale revoke authority");
    assert!(is_forbidden(&list));
    assert!(is_forbidden(&grant));
    assert!(is_forbidden(&revoke));
    assert_eq!(
        grant_count(&w, namespace).await,
        1,
        "stale revoke changed nothing"
    );
}

#[tokio::test]
async fn a_failed_audit_rolls_back_the_grant() {
    let w = world().await;
    let service = grants(&w);
    let namespace = w.namespace.id();
    let developer = w.member("audit-rollback", Role::Developer).await;
    let scope = service
        .prepare(&w.admin, namespace, &ctx())
        .await
        .expect("scope");

    sqlx::query(
        "CREATE FUNCTION reject_reveal_grant_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action = 'payload.reveal_granted' THEN \
           RAISE EXCEPTION 'test audit failure'; END IF; RETURN NEW; END; $$",
    )
    .execute(&w.db.pool)
    .await
    .expect("failure function");
    sqlx::query(
        "CREATE TRIGGER reject_reveal_grant_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_reveal_grant_audit()",
    )
    .execute(&w.db.pool)
    .await
    .expect("failure trigger");

    assert!(matches!(
        service
            .grant(&scope, developer.subject().as_str(), &ctx())
            .await,
        Err(RevealGrantError::Application(_))
    ));
    assert_eq!(grant_count(&w, namespace).await, 0);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
         AND action='payload.reveal_granted'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit count");
    assert_eq!(audit, 0);
}

#[tokio::test]
async fn an_effective_grant_read_fences_revoke_until_disclosure_commit() {
    let w = world().await;
    let service = Arc::new(grants(&w));
    let namespace = w.namespace.id();
    let developer = w.member("reveal-reader", Role::Developer).await;
    let scope = service
        .prepare(&w.admin, namespace, &ctx())
        .await
        .expect("scope");
    service
        .grant(&scope, developer.subject().as_str(), &ctx())
        .await
        .expect("seed grant");

    let mut disclosure = w.db.pool.begin().await.expect("disclosure transaction");
    assert!(
        has_effective_payload_reveal_grant(
            &mut disclosure,
            w.admin.tenant_id(),
            namespace,
            developer.subject(),
        )
        .await
        .expect("effective grant")
    );

    let revoke_service = Arc::clone(&service);
    let revoke_scope = scope.clone();
    let revoke_subject = developer.subject().as_str().to_owned();
    let mut revoke = tokio::spawn(async move {
        revoke_service
            .revoke(&revoke_scope, &revoke_subject, &ctx())
            .await
    });
    crate::support::eventually("revoke waits for the disclosure grant fence", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted)",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("waiting advisory lock")
        .then_some(())
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut revoke)
            .await
            .is_err(),
        "revoke committed while disclosure still held the shared fence"
    );

    disclosure
        .commit()
        .await
        .expect("disclosure audit/commit boundary");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), revoke)
            .await
            .expect("revoke unblocked")
            .expect("revoke task")
            .expect("revoke result")
    );

    let mut after = w.db.pool.begin().await.expect("next request");
    assert!(
        !has_effective_payload_reveal_grant(
            &mut after,
            w.admin.tenant_id(),
            namespace,
            developer.subject(),
        )
        .await
        .expect("grant after revoke")
    );
    after.rollback().await.expect("close next request");

    let audit_actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action IN ('payload.reveal_granted', 'payload.reveal_revoked') ORDER BY created_at",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(format!("{namespace}/{}", developer.subject().as_str()))
    .fetch_all(&w.db.pool)
    .await
    .expect("grant/revoke audits");
    assert_eq!(
        audit_actions,
        vec!["payload.reveal_granted", "payload.reveal_revoked"]
    );
}

#[tokio::test]
async fn http_repeated_delete_returns_not_found_and_responses_are_not_cached() {
    use reqwest::{Method, StatusCode};
    let mut options = crate::support::api::Options::default();
    options.limits.max_body_bytes = 1024;
    let api = crate::support::api::start_with(options).await;
    api.tenant("grants-http", "admin").await;
    let workspace = api.create_workspace("admin", "grants-http", "work").await;
    let namespace = api
        .create_namespace("admin", "grants-http", &workspace, "sandbox", "dev")
        .await;
    let path = format!("/namespaces/{namespace}/payload-reveal-grants/admin");
    let put = api
        .as_user(Method::PUT, &path, "admin", "grants-http")
        .send()
        .await
        .expect("grant HTTP");
    let oversized = api
        .as_user(Method::PUT, &path, "admin", "grants-http")
        .body(vec![b'x'; 1025])
        .send()
        .await
        .expect("oversized grant HTTP");
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(put.status(), StatusCode::OK);
    assert_eq!(put.headers()["cache-control"], "no-store");
    for expected in [StatusCode::OK, StatusCode::NOT_FOUND] {
        let response = api
            .as_user(Method::DELETE, &path, "admin", "grants-http")
            .send()
            .await
            .expect("revoke HTTP");
        assert_eq!(response.status(), expected);
    }
}

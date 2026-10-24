//! Grant resource wire contract and built-in-role authority (T016-032).
use reqwest::{Method, StatusCode};
use sagat_domain::identity::{Membership, Role};
use serde_json::Value;
use uuid::Uuid;

use super::support::{self, api::TestApi, contract::OpenApi};

const TENANT: &str = "grant-contract";
const ADMIN: &str = "admin";

async fn call(
    api: &TestApi,
    actor: &str,
    method: Method,
    path: &str,
    template: &str,
    expected: u16,
) -> Value {
    let response = api
        .as_user(method.clone(), path, actor, TENANT)
        .send()
        .await
        .expect("grant resource HTTP");
    assert_eq!(response.status().as_u16(), expected);
    if response.status().is_success() {
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let body = support::api::json_of(response).await;
    OpenApi::load()
        .validate_body(method.as_str(), template, expected, &body)
        .expect("documented grant response")
        .expect("grant response schema");
    body
}

#[tokio::test]
async fn manager_only_grants_have_closed_errors_idempotent_put_and_a_real_http_cap() {
    let api = support::api::start().await;
    let tenant = api.tenant(TENANT, ADMIN).await;
    let workspace = api.create_workspace(ADMIN, TENANT, "work").await;
    let namespace = api
        .create_namespace(ADMIN, TENANT, &workspace, "payments", "dev")
        .await;
    for (subject, role) in [
        ("grant-viewer", Role::Viewer),
        ("developer", Role::Developer),
        ("operator", Role::Operator),
        ("worker", Role::Worker),
        ("runner", Role::Runner),
        ("database-runner", Role::DatabaseRunner),
    ] {
        api.owner_store
            .insert_membership(&Membership::restore(
                tenant,
                support::subject(subject),
                role,
                support::now(),
            ))
            .await
            .expect("fixture membership");
    }
    let list_template = "/namespaces/{namespaceId}/payload-reveal-grants";
    let item_template = "/namespaces/{namespaceId}/payload-reveal-grants/{subject}";
    let list = format!("/namespaces/{namespace}/payload-reveal-grants");
    let developer = format!("{list}/developer");
    for actor in [
        "grant-viewer",
        "developer",
        "operator",
        "worker",
        "runner",
        "database-runner",
    ] {
        for (method, path, template) in [
            (Method::GET, list.as_str(), list_template),
            (Method::PUT, developer.as_str(), item_template),
            (Method::DELETE, developer.as_str(), item_template),
        ] {
            let body = call(&api, actor, method, path, template, 403).await;
            assert_eq!(body["code"], "FORBIDDEN");
        }
    }
    for (subject, reason) in [
        ("unknown-contract-subject", "UNKNOWN_SUBJECT"),
        ("grant-viewer", "ROLE_CANNOT_REVEAL"),
    ] {
        let body = call(
            &api,
            ADMIN,
            Method::PUT,
            &format!("{list}/{subject}"),
            item_template,
            422,
        )
        .await;
        assert_eq!(body["details"]["reason"], reason);
        assert!(
            !body.to_string().contains(subject),
            "grant refusal reflected its subject"
        );
    }
    call(
        &api,
        ADMIN,
        Method::PUT,
        &format!("{list}/{ADMIN}"),
        item_template,
        200,
    )
    .await;
    let first = call(&api, ADMIN, Method::PUT, &developer, item_template, 200).await;
    let repeated = call(&api, ADMIN, Method::PUT, &developer, item_template, 200).await;
    assert_eq!(first, repeated);
    let page = call(
        &api,
        ADMIN,
        Method::GET,
        &format!("{list}?limit=1"),
        list_template,
        200,
    )
    .await;
    assert_eq!(page["items"].as_array().expect("grant items").len(), 1);
    let cursor = page["nextCursor"].as_str().expect("next grant page");
    let next = call(
        &api,
        ADMIN,
        Method::GET,
        &format!("{list}?limit=1&cursor={cursor}"),
        list_template,
        200,
    )
    .await;
    assert_eq!(next["items"][0]["subject"], "developer");
    call(&api, ADMIN, Method::DELETE, &developer, item_template, 200).await;
    call(&api, ADMIN, Method::DELETE, &developer, item_template, 404).await;

    // The application/PG race suite creates the cap through its audited service. This fixture fills
    // the remaining slots directly to isolate the public GRANT_LIMIT response mapping.
    sqlx::query("INSERT INTO memberships (tenant_id,subject,role,created_at) SELECT $1,'grant-cap-'||i::text,$2,clock_timestamp() FROM generate_series(1,255) i")
        .bind(tenant.as_uuid()).bind(Role::Developer.as_str()).execute(&api.db.pool).await.expect("cap members");
    sqlx::query("INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) SELECT $1,$2,subject,$3,clock_timestamp() FROM memberships WHERE tenant_id=$1 AND subject LIKE 'grant-cap-%'")
        .bind(tenant.as_uuid()).bind(Uuid::parse_str(&namespace).expect("namespace UUID")).bind(ADMIN)
        .execute(&api.db.pool).await.expect("cap grants fixture");
    let capped = call(
        &api,
        ADMIN,
        Method::PUT,
        &developer,
        item_template,
        StatusCode::TOO_MANY_REQUESTS.as_u16(),
    )
    .await;
    assert_eq!(capped["code"], "QUOTA_EXCEEDED");
    assert_eq!(capped["details"]["reason"], "GRANT_LIMIT");
}

#[tokio::test]
async fn the_runtime_role_cannot_update_grants_or_bypass_their_composite_tenant_keys() {
    let db = support::migrated_db().await;
    let privileges: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege('sagat_app','payload_reveal_grants','SELECT'), has_table_privilege('sagat_app','payload_reveal_grants','INSERT'), has_table_privilege('sagat_app','payload_reveal_grants','DELETE'), has_table_privilege('sagat_app','payload_reveal_grants','UPDATE')",
    ).fetch_one(&db.pool).await.expect("runtime grant privileges");
    assert_eq!(privileges, (true, true, true, false));
    let keys: Vec<(String, String)> = sqlx::query_as(
        "SELECT conname,pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='payload_reveal_grants'::regclass AND contype='f' ORDER BY conname",
    ).fetch_all(&db.pool).await.expect("composite grant foreign keys");
    assert_eq!(keys.len(), 2);
    assert!(
        keys.iter()
            .all(|(_, definition)| definition.contains("FOREIGN KEY (tenant_id,"))
    );
    assert!(keys.iter().any(
        |(_, definition)| definition.contains("REFERENCES memberships")
            && definition.contains("ON DELETE CASCADE")
    ));
}

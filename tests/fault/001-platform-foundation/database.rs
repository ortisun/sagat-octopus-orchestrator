//! The database misbehaves while the API serves traffic.

use std::time::Duration;

use reqwest::Method;
use serde_json::json;

use crate::support::api::{Options, json_of, start, start_with};

async fn lock_table(
    api: &crate::support::api::TestApi,
    table: &str,
) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    let mut blocker = api.db.pool.acquire().await.expect("blocker");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    // Table names here are test constants, not input.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"
    )))
    .execute(&mut *blocker)
    .await
    .expect("lock");
    blocker
}

#[tokio::test]
async fn a_stuck_database_yields_a_retryable_503_and_service_resumes_without_a_restart() {
    let api = start_with(Options {
        statement_timeout: Duration::from_millis(300),
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let mut blocker = lock_table(&api, "memberships").await; // authentication reads memberships

    let started = std::time::Instant::now();
    let during = api.get_as("/auth/context", "alice", "acme").await;

    assert_eq!(during.status(), 503);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "gave up quickly, took {:?}",
        started.elapsed()
    );
    let body = json_of(during).await;
    assert_eq!(body["code"], "UNAVAILABLE");
    assert_eq!(body["retryable"], true);

    // Liveness is independent of the database; readiness is not.
    assert_eq!(
        api.request(Method::GET, "/health")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );

    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");
    drop(blocker);
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200
    );
}

#[tokio::test]
async fn readiness_reports_the_database_and_recovers() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    assert_eq!(
        api.request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );

    // Take the database away from this API: cut its connections and make new ones impossible.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE \"{}\" ALLOW_CONNECTIONS false",
        api.db.name
    )))
    .execute(
        &sqlx::PgPool::connect(&api.db.admin_url)
            .await
            .expect("admin"),
    )
    .await
    .expect("block connections");
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()")
        .execute(&api.db.pool)
        .await
        .ok(); // may itself be cut off; that is the point

    let mut saw_unavailable = false;
    let mut statuses = Vec::new();
    for _ in 0..10 {
        let ready = api
            .request(Method::GET, "/ready")
            .send()
            .await
            .expect("send");
        statuses.push(ready.status().as_u16());
        if ready.status() == 503 {
            saw_unavailable = true;
            assert_eq!(json_of(ready).await["code"], "UNAVAILABLE");
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        saw_unavailable,
        "readiness must go red while the database is unreachable; statuses seen: {statuses:?}"
    );

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE \"{}\" ALLOW_CONNECTIONS true",
        api.db.name
    )))
    .execute(
        &sqlx::PgPool::connect(&api.db.admin_url)
            .await
            .expect("admin"),
    )
    .await
    .expect("restore");
    let mut recovered = false;
    for _ in 0..30 {
        if api
            .request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status()
            == 200
        {
            recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        recovered,
        "readiness must recover on its own once the database is back"
    );
}

#[tokio::test]
async fn killing_every_connection_never_produces_a_500_and_nothing_is_lost() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;

    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()")
        .execute(&api.db.pool)
        .await
        .expect("terminate");

    let mut last = 0;
    for _ in 0..10 {
        let response = api
            .get_as(&format!("/workspaces/{ws}"), "alice", "acme")
            .await;
        last = response.status().as_u16();
        assert!(
            matches!(last, 200 | 503),
            "an infrastructure failure must be 503, never {last}"
        );
        if last == 200 {
            assert_eq!(json_of(response).await["slug"], "platform", "data intact");
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(last, 200, "the pool re-establishes connections by itself");
}

#[tokio::test]
async fn retrying_a_create_after_an_ambiguous_response_is_a_409_and_the_original_is_intact() {
    // Documents the known idempotency gap (research.md G7): without an Idempotency-Key a retry of a
    // create that actually succeeded is reported as a conflict, and never creates a duplicate.
    let api = start().await;
    api.tenant("acme", "alice").await;
    let body = json!({"slug": "platform"});

    let first = api.post_as("/workspaces", "alice", "acme", &body).await;
    assert_eq!(first.status(), 201);
    let original = json_of(first).await;
    let retry = api.post_as("/workspaces", "alice", "acme", &body).await;

    assert_eq!(retry.status(), 409);
    let listed = json_of(api.get_as("/workspaces", "alice", "acme").await).await;
    assert_eq!(
        listed["items"],
        json!([original]),
        "exactly one workspace, unchanged"
    );
}

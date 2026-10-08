//! Overload behaviour (gap G13): per-tenant budgets, load shedding, cached readiness.

use std::time::Duration;

use reqwest::Method;
use sagat_api_http::{HttpLimits, RateLimitConfig};

use crate::support::api::{Options, json_of, start_with};

fn tight_budget() -> Options {
    Options {
        // 3 requests of burst, then one more per second.
        rate_limit: RateLimitConfig {
            per_second: 1.0,
            burst: 3.0,
        },
        ..Options::default()
    }
}

#[tokio::test]
async fn a_tenant_over_its_budget_gets_429_while_other_tenants_are_served() {
    let api = start_with(tight_budget()).await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;

    for i in 0..3 {
        assert_eq!(
            api.get_as("/auth/context", "alice", "acme").await.status(),
            200,
            "request {i}"
        );
    }
    let limited = api.get_as("/auth/context", "alice", "acme").await;

    assert_eq!(limited.status(), 429);
    let retry_after: u64 = limited.headers()["retry-after"]
        .to_str()
        .expect("ascii")
        .parse()
        .expect("seconds");
    assert!(
        (1..=2).contains(&retry_after),
        "Retry-After must be a positive whole number of seconds: {retry_after}"
    );
    let body = json_of(limited).await;
    assert_eq!(body["code"], "RATE_LIMITED");
    assert_eq!(body["retryable"], true);

    // Another tenant has its own budget.
    assert_eq!(
        api.get_as("/auth/context", "bob", "globex").await.status(),
        200
    );
    // The same tenant recovers once tokens refill.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200
    );
}

#[tokio::test]
async fn anonymous_traffic_cannot_spend_a_tenants_budget() {
    let api = start_with(tight_budget()).await;
    api.tenant("acme", "alice").await;

    // Twenty anonymous / forged requests are rejected as 401 and charge nobody.
    for _ in 0..20 {
        assert_eq!(
            api.request(Method::GET, "/auth/context")
                .send()
                .await
                .expect("send")
                .status(),
            401
        );
        let forged = api
            .request(Method::GET, "/auth/context")
            .bearer_auth("not.a.jwt")
            .send()
            .await
            .expect("send");
        assert_eq!(forged.status(), 401);
    }

    for i in 0..3 {
        assert_eq!(
            api.get_as("/auth/context", "alice", "acme").await.status(),
            200,
            "alice still has her full budget ({i})"
        );
    }
}

#[tokio::test]
async fn probes_are_exempt_from_the_tenant_budget() {
    let api = start_with(tight_budget()).await;
    for _ in 0..30 {
        assert_eq!(
            api.request(Method::GET, "/health")
                .send()
                .await
                .expect("send")
                .status(),
            200
        );
    }
}

#[tokio::test]
async fn when_saturated_the_api_sheds_load_at_once_and_probes_keep_answering() {
    let api = start_with(Options {
        limits: HttpLimits {
            max_in_flight: 1,
            ..HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let mut blocker = api.db.pool.acquire().await.expect("conn");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE workspaces IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");

    // Request 1 occupies the only slot (it blocks on the locked table).
    let client = api.client.clone();
    let url = format!("{}/workspaces", api.base);
    let token = api.idp.token("alice", "acme").build();
    let occupant = tokio::spawn(async move {
        client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .expect("send")
            .status()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let started = std::time::Instant::now();
    let shed = api.get_as("/namespaces", "alice", "acme").await;
    assert_eq!(shed.status(), 503, "no free slot");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "refused at once, not queued: {:?}",
        started.elapsed()
    );
    assert_eq!(json_of(shed).await["retryable"], true);
    assert_eq!(
        api.request(Method::GET, "/health")
            .send()
            .await
            .expect("send")
            .status(),
        200,
        "liveness is exempt"
    );

    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");
    assert_eq!(
        occupant.await.expect("join").as_u16(),
        200,
        "the request in the slot finished normally"
    );
    assert_eq!(
        api.get_as("/namespaces", "alice", "acme").await.status(),
        200,
        "and the slot is free again"
    );
}

#[tokio::test]
async fn a_flood_of_readiness_probes_costs_one_query_per_second() {
    let api = start_with(Options::default()).await;
    assert_eq!(
        api.request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );

    // Take the database away. The cached "ready" answer is served for up to a second ...
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
    .expect("block");
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()")
        .execute(&api.db.pool)
        .await
        .ok();
    assert_eq!(
        api.request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200,
        "served from the cache"
    );

    // ... and the truth shows up as soon as the cache expires.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        api.request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        503
    );
}

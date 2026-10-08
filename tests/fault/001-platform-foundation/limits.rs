//! Overload behaviour (gap G13): per-tenant budgets, load shedding, cached readiness.

use std::time::Duration;

use reqwest::Method;
use sagat_api_http::{HttpLimits, RateLimitConfig};
use sagat_telemetry::Metrics;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::api::{Options, TestApi, json_of, start_with};

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

fn tight_subject_budget() -> Options {
    Options {
        // 5 requests of burst per subject, then one more per second.
        subject_rate_limit: RateLimitConfig {
            per_second: 1.0,
            burst: 5.0,
        },
        ..Options::default()
    }
}

#[tokio::test]
async fn a_valid_token_without_membership_is_throttled_before_it_costs_database_lookups() {
    // Review finding 3: a non-member with a valid IdP token (or an unknown tenant claim) never
    // reached the tenant limiter, and every request cost two lookups (300 requests, 600 lookups).
    let api = start_with(tight_subject_budget()).await;
    api.tenant("acme", "alice").await;
    let baseline = api.authentication_lookups();

    let mut statuses = Vec::new();
    for _ in 0..60 {
        // `mallory` is a perfectly valid subject of the identity provider, but not a member.
        statuses.push(
            api.get_as("/auth/context", "mallory", "acme")
                .await
                .status()
                .as_u16(),
        );
    }

    let refused = statuses.iter().filter(|s| **s == 401).count();
    let limited = statuses.iter().filter(|s| **s == 429).count();
    assert_eq!(refused + limited, 60, "{statuses:?}");
    assert!(
        (5..=7).contains(&refused),
        "only the burst (plus at most the tokens refilled meanwhile) reaches authentication: {statuses:?}"
    );
    let lookups = api.authentication_lookups() - baseline;
    assert!(
        lookups <= 2 * refused,
        "{lookups} lookups for {refused} authenticated attempts: throttled requests must not query"
    );

    // The same holds for a tenant claim that does not exist, and the envelope is the usual one.
    for _ in 0..5 {
        api.get_as("/auth/context", "trudy", "no-such-tenant").await;
    }
    let limited = api.get_as("/auth/context", "trudy", "no-such-tenant").await;
    assert_eq!(limited.status(), 429);
    assert!(limited.headers().contains_key("retry-after"));
    let body = json_of(limited).await;
    assert_eq!(body["code"], "RATE_LIMITED");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
async fn one_member_cannot_exhaust_the_tenant_budget_on_their_own() {
    // Review finding 7: a single viewer could spend the whole tenant bucket and lock out the
    // tenant's administrators.
    let api = start_with(Options {
        rate_limit: RateLimitConfig {
            per_second: 0.5,
            burst: 20.0,
        },
        subject_rate_limit: RateLimitConfig {
            per_second: 0.5,
            burst: 6.0,
        },
        ..Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    add_member(&api, tenant, "vince", "viewer").await;

    let mut viewer = Vec::new();
    for _ in 0..40 {
        viewer.push(
            api.get_as("/auth/context", "vince", "acme")
                .await
                .status()
                .as_u16(),
        );
    }
    assert!(
        viewer.iter().filter(|s| **s == 200).count() <= 7,
        "the viewer is held to its own budget: {viewer:?}"
    );

    // The administrator still has most of the tenant bucket.
    for i in 0..6 {
        assert_eq!(
            api.get_as("/auth/context", "alice", "acme").await.status(),
            200,
            "administrator request {i}"
        );
    }
}

/// Adds a membership as the database owner.
async fn add_member(
    api: &crate::support::api::TestApi,
    tenant: sagat_domain::identity::TenantId,
    subject: &str,
    role: &str,
) {
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) VALUES ($1, $2, $3, now())",
    )
    .bind(tenant.as_uuid())
    .bind(subject)
    .bind(role)
    .execute(&api.db.pool)
    .await
    .expect("membership");
}

/// Sends the head of an authenticated `POST /v1/workspaces` that announces a 64-byte body and never
/// sends it: the request holds its slot until the server gives up on it.
async fn stalled_post(api: &TestApi, token: &str) -> TcpStream {
    let authority = api
        .base
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("authority");
    let mut stream = TcpStream::connect(authority).await.expect("connect");
    let head = format!(
        "POST /v1/workspaces HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: 64\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.expect("head");
    stream
}

/// The status code the server answers with within `limit`, or `None` if it stays silent or closes.
async fn status_within(stream: &mut TcpStream, limit: Duration) -> Option<u16> {
    let mut received = Vec::new();
    let read = async {
        let mut chunk = [0u8; 512];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    received.extend_from_slice(&chunk[..n]);
                    if received.windows(2).any(|w| w == b"\r\n") {
                        return;
                    }
                }
            }
        }
    };
    tokio::time::timeout(limit, read).await.ok()?;
    String::from_utf8_lossy(&received)
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[tokio::test]
async fn stalled_uploads_of_one_tenant_cannot_starve_the_others() {
    // Review finding 2: the global slot is taken before the body is read, so N authenticated
    // requests that announce a body and never send it starved every tenant for 15 s each.
    let metrics = Metrics::global();
    let api = start_with(Options {
        limits: HttpLimits {
            max_in_flight: 4,
            max_in_flight_per_tenant: 2,
            body_idle_timeout: Duration::from_secs(1),
            ..HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let token = api.idp.token("alice", "acme").build();
    let mut stalled = vec![
        stalled_post(&api, &token).await,
        stalled_post(&api, &token).await,
    ];

    // Tenant A is at its cap once both stalled requests have taken their slots (a moment after the
    // connections are written, longer on a loaded machine): one more request is refused at once,
    // with the usual envelope.
    let mut refused = api.get_as("/auth/context", "alice", "acme").await;
    for _ in 0..60 {
        if refused.status() == 429 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        refused = api.get_as("/auth/context", "alice", "acme").await;
    }
    assert_eq!(refused.status(), 429);
    assert!(refused.headers().contains_key("retry-after"));
    let body = json_of(refused).await;
    assert_eq!(body["code"], "RATE_LIMITED");
    assert_eq!(body["retryable"], true);
    assert!(
        metrics
            .render()
            .contains("http_tenant_concurrency_rejected_total"),
        "the refusal is counted"
    );

    // Tenant B is untouched: two global slots are still free.
    assert_eq!(
        api.get_as("/auth/context", "bob", "globex").await.status(),
        200
    );
    assert_eq!(
        api.post_as(
            "/workspaces",
            "bob",
            "globex",
            &serde_json::json!({ "slug": "platform" })
        )
        .await
        .status(),
        201
    );

    // The stalled requests are cut at the body idle timeout (1 s), long before the 15 s request
    // timeout, with a retryable 408.
    for connection in &mut stalled {
        assert_eq!(
            status_within(connection, Duration::from_secs(4)).await,
            Some(408),
            "a stalled body must not hold its slot until the request timeout"
        );
    }
    // And the slots are free again.
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200
    );
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
    let retry_after: u64 = shed.headers()["retry-after"]
        .to_str()
        .expect("ascii")
        .parse()
        .expect("Retry-After is a whole number of seconds");
    assert!(
        (3..=8).contains(&retry_after),
        "a small jittered delay, not a constant that synchronises every client: {retry_after}"
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

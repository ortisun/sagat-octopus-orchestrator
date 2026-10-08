//! Live conformance of the Identity operations to `openapi.yaml` (T001-002, T001-031).

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_api_http::error::ErrorCode;
use sagat_api_http::{HttpLimits, RateLimitConfig};
use sagat_domain::identity::{Membership, Role};
use serde_json::json;

use crate::support::api::{Options, start, start_with};
use crate::support::contract::{As, OpenApi, Scenario, check, scenario};
use crate::support::{now, subject};

#[tokio::test]
async fn every_identity_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme_id = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    // A developer in acme: authenticated, but not allowed to create scopes.
    api.owner_store
        .insert_membership(&Membership::restore(
            acme_id,
            subject("dan"),
            Role::Developer,
            now(),
        ))
        .await
        .expect("grant developer");

    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "dev", "dev")
        .await;
    let foreign_ws = api.create_workspace("bob", "globex", "other").await;
    let missing = "019b5d6e-0000-7000-8000-000000000000";
    let alice = As::User("alice", "acme");
    let dan = As::User("dan", "acme");

    let ws_t = "/workspaces/{workspaceId}";
    let ns_t = "/namespaces/{namespaceId}";
    let create_ns_t = "/workspaces/{workspaceId}/namespaces";
    let scenarios = vec![
        scenario("GET", "/health", "/health", As::Anonymous, None, 200),
        scenario("GET", "/ready", "/ready", As::Anonymous, None, 200),
        scenario(
            "GET",
            "/auth/context",
            "/auth/context",
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/auth/context",
            "/auth/context",
            As::Anonymous,
            None,
            401,
        ),
        scenario(
            "GET",
            "/auth/context",
            "/auth/context",
            As::Garbage,
            None,
            401,
        ),
        scenario(
            "GET",
            "/workspaces",
            "/workspaces",
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/workspaces",
            "/workspaces?limit=1",
            dan.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/workspaces",
            "/workspaces?cursor=garbage",
            alice.clone(),
            None,
            422,
        ),
        scenario(
            "GET",
            "/workspaces",
            "/workspaces",
            As::Anonymous,
            None,
            401,
        ),
        scenario(
            "POST",
            "/workspaces",
            "/workspaces",
            alice.clone(),
            Some(json!({"slug": "second"})),
            201,
        ),
        scenario(
            "POST",
            "/workspaces",
            "/workspaces",
            alice.clone(),
            Some(json!({"slug": "second"})),
            409,
        ),
        scenario(
            "POST",
            "/workspaces",
            "/workspaces",
            alice.clone(),
            Some(json!({"slug": "Bad Slug"})),
            422,
        ),
        scenario(
            "POST",
            "/workspaces",
            "/workspaces",
            dan.clone(),
            Some(json!({"slug": "mine"})),
            403,
        ),
        scenario(
            "POST",
            "/workspaces",
            "/workspaces",
            As::Anonymous,
            Some(json!({"slug": "mine"})),
            401,
        ),
        scenario(
            "GET",
            ws_t,
            &format!("/workspaces/{ws}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            ws_t,
            &format!("/workspaces/{missing}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            ws_t,
            &format!("/workspaces/{foreign_ws}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            ws_t,
            &format!("/workspaces/{ws}"),
            As::Anonymous,
            None,
            401,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{ws}/namespaces"),
            alice.clone(),
            Some(json!({"slug": "staging-1", "environment": "staging"})),
            201,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{ws}/namespaces"),
            alice.clone(),
            Some(json!({"slug": "staging-1", "environment": "prod"})),
            409,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{ws}/namespaces"),
            alice.clone(),
            Some(json!({"slug": "qa-1", "environment": "qa"})),
            422,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{foreign_ws}/namespaces"),
            alice.clone(),
            Some(json!({"slug": "x-1", "environment": "dev"})),
            404,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{ws}/namespaces"),
            dan.clone(),
            Some(json!({"slug": "dan-1", "environment": "dev"})),
            403,
        ),
        scenario(
            "POST",
            create_ns_t,
            &format!("/workspaces/{ws}/namespaces"),
            As::Anonymous,
            Some(json!({"slug": "a-1", "environment": "dev"})),
            401,
        ),
        scenario(
            "GET",
            "/namespaces",
            "/namespaces",
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/namespaces",
            "/namespaces?limit=1&cursor=garbage",
            alice.clone(),
            None,
            422,
        ),
        scenario(
            "GET",
            "/namespaces",
            "/namespaces",
            As::Anonymous,
            None,
            401,
        ),
        scenario(
            "GET",
            ns_t,
            &format!("/namespaces/{ns}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            ns_t,
            &format!("/namespaces/{missing}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            ns_t,
            &format!("/namespaces/{ns}"),
            As::Garbage,
            None,
            401,
        ),
    ];

    let mut exercised: BTreeSet<(String, String, u16)> = BTreeSet::new();
    for s in &scenarios {
        exercised.insert(check(&api, &spec, s).await);
    }

    // Every operation of the contract has at least one successful scenario.
    let covered: BTreeSet<(String, String)> = exercised
        .iter()
        .filter(|(_, _, st)| (200..300).contains(st))
        .map(|(m, t, _)| (m.clone(), t.clone()))
        .collect();
    for (method, template) in spec.operations_tagged("Identity") {
        assert!(
            covered.contains(&(method.clone(), template.clone())),
            "{method} {template} has no conformance scenario"
        );
    }
}

#[tokio::test]
async fn the_overload_and_outage_responses_conform_too() {
    let spec = OpenApi::load();

    // 429 with Retry-After
    let limited = start_with(Options {
        rate_limit: RateLimitConfig {
            per_second: 0.5,
            burst: 1.0,
        },
        ..Options::default()
    })
    .await;
    limited.tenant("acme", "alice").await;
    let call = scenario(
        "GET",
        "/auth/context",
        "/auth/context",
        As::User("alice", "acme"),
        None,
        200,
    );
    check(&limited, &spec, &call).await;
    let over = Scenario {
        expect: 429,
        ..call
    };
    check(&limited, &spec, &over).await;

    // 503 when the database cannot answer, and 408 when the request times out
    let stuck = start_with(Options {
        statement_timeout: Duration::from_millis(300),
        limits: HttpLimits {
            request_timeout: Duration::from_secs(20),
            ..HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    stuck.tenant("acme", "alice").await;
    let mut blocker = stuck.db.pool.acquire().await.expect("conn");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE memberships IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");
    let down = scenario(
        "GET",
        "/auth/context",
        "/auth/context",
        As::User("alice", "acme"),
        None,
        503,
    );
    check(&stuck, &spec, &down).await;
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");

    let slow = start_with(Options {
        limits: HttpLimits {
            request_timeout: Duration::from_millis(400),
            ..HttpLimits::default()
        },
        statement_timeout: Duration::from_secs(10),
        ..Options::default()
    })
    .await;
    slow.tenant("acme", "alice").await;
    let mut blocker = slow.db.pool.acquire().await.expect("conn");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE memberships IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");
    let timed_out = scenario(
        "GET",
        "/workspaces",
        "/workspaces",
        As::User("alice", "acme"),
        None,
        408,
    );
    check(&slow, &spec, &timed_out).await;
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");
}

#[test]
fn the_error_codes_of_the_contract_are_exactly_the_ones_the_code_can_emit() {
    // The exhaustive match makes adding a variant a compile error here until the contract follows.
    fn name(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::BadRequest => "BAD_REQUEST",
            ErrorCode::Unauthenticated => "UNAUTHENTICATED",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::ValidationFailed => "VALIDATION_FAILED",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::Unavailable => "UNAVAILABLE",
            ErrorCode::Internal => "INTERNAL",
        }
    }
    let in_code: BTreeSet<String> = [
        ErrorCode::BadRequest,
        ErrorCode::Unauthenticated,
        ErrorCode::Forbidden,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::ValidationFailed,
        ErrorCode::RateLimited,
        ErrorCode::Unavailable,
        ErrorCode::Internal,
    ]
    .into_iter()
    .map(|c| {
        let wire = serde_json::to_value(c).expect("serialize");
        assert_eq!(wire, name(c), "serialized form of {c:?}");
        name(c).to_owned()
    })
    .collect();
    let in_contract: BTreeSet<String> = OpenApi::load().error_codes().into_iter().collect();
    assert_eq!(in_code, in_contract);
}

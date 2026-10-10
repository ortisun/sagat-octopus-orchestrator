//! US02 — Authenticate an SDK client (FR-001-002, FR-001-007).
//!
//! Given a valid token with a tenant scope, when the client queries namespaces, then the API
//! returns only the authorized resources and a trace id.

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::api::{json_of, start};
use crate::support::oidc::{TestKey, now_secs};
use crate::{trace_id, without_trace};

#[tokio::test]
async fn the_auth_context_describes_the_caller_and_carries_the_trace_id() {
    let api = start().await;
    api.tenant("acme", "alice").await;

    let response = api.get_as("/auth/context", "alice", "acme").await;

    assert_eq!(response.status(), 200);
    let header_trace = trace_id(&response);
    let body = json_of(response).await;
    assert_eq!(body["tenant"], "acme");
    assert_eq!(body["subject"], "alice");
    assert_eq!(body["role"], "admin");
    assert_eq!(body["traceId"], header_trace);
    let permissions: Vec<&str> = body["permissions"]
        .as_array()
        .expect("array")
        .iter()
        .map(|p| p.as_str().expect("str"))
        .collect();
    assert!(permissions.contains(&"workspace:create") && permissions.contains(&"audit:read"));
}

#[tokio::test]
async fn an_sdk_client_sees_only_the_namespaces_of_its_own_tenant() {
    let api = start().await;
    api.tenant("acme", "sdk-client").await;
    api.tenant("globex", "bob").await;
    let ws_a = api.create_workspace("sdk-client", "acme", "platform").await;
    api.create_namespace("sdk-client", "acme", &ws_a, "a-dev", "dev")
        .await;
    api.create_namespace("sdk-client", "acme", &ws_a, "a-prod", "prod")
        .await;
    let ws_b = api.create_workspace("bob", "globex", "platform").await;
    api.create_namespace("bob", "globex", &ws_b, "b-dev", "dev")
        .await;

    let response = api.get_as("/namespaces", "sdk-client", "acme").await;

    assert_eq!(response.status(), 200);
    trace_id(&response);
    let page = json_of(response).await;
    let mut slugs: Vec<&str> = page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|n| n["slug"].as_str().expect("slug"))
        .collect();
    slugs.sort_unstable();
    assert_eq!(
        slugs,
        ["a-dev", "a-prod"],
        "tenant globex's namespace must not appear"
    );
}

#[tokio::test]
async fn cursor_pagination_walks_every_item_once() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    for i in 0..5 {
        api.create_namespace("alice", "acme", &ws, &format!("ns-{i}"), "dev")
            .await;
    }

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let mut path = "/namespaces?limit=2".to_owned();
        if let Some(c) = &cursor {
            path.push_str(&format!("&cursor={c}"));
        }
        let page = json_of(api.get_as(&path, "alice", "acme").await).await;
        seen.extend(
            page["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(|n| n["slug"].as_str().expect("slug").to_owned()),
        );
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }

    assert_eq!(seen, ["ns-0", "ns-1", "ns-2", "ns-3", "ns-4"]);

    let bad = api
        .get_as("/namespaces?cursor=garbage", "alice", "acme")
        .await;
    assert_eq!(bad.status(), 422);
    let not_a_number = api.get_as("/namespaces?limit=abc", "alice", "acme").await;
    assert_eq!(not_a_number.status(), 400);
}

#[tokio::test]
async fn every_way_of_failing_authentication_gets_the_same_401() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;

    let attacker = TestKey::generate();
    let mut attempts: Vec<(&str, reqwest::RequestBuilder)> = Vec::new();
    attempts.push(("no credentials", api.request(Method::GET, "/auth/context")));
    attempts.push((
        "garbage token",
        api.request(Method::GET, "/auth/context")
            .bearer_auth("not.a.jwt"),
    ));
    attempts.push((
        "expired",
        api.request(Method::GET, "/auth/context")
            .bearer_auth(api.idp.token("alice", "acme").issued(900, 300).build()),
    ));
    attempts.push((
        "forged signature",
        api.request(Method::GET, "/auth/context").bearer_auth(
            api.idp
                .token("alice", "acme")
                .signed_with(attacker.with_kid("whatever"))
                .build(),
        ),
    ));
    attempts.push((
        "wrong audience",
        api.request(Method::GET, "/auth/context").bearer_auth(
            api.idp
                .token("alice", "acme")
                .claim("aud", json!("other"))
                .build(),
        ),
    ));
    attempts.push((
        "unknown tenant",
        api.request(Method::GET, "/auth/context")
            .bearer_auth(api.idp.token("alice", "nowhere").build()),
    ));
    attempts.push((
        "no membership",
        api.request(Method::GET, "/auth/context")
            .bearer_auth(api.idp.token("stranger", "acme").build()),
    ));
    attempts.push((
        "member of another tenant",
        api.request(Method::GET, "/auth/context")
            .bearer_auth(api.idp.token("bob", "acme").build()),
    ));
    attempts.push((
        "wrong scheme",
        api.request(Method::GET, "/auth/context").header(
            "authorization",
            format!("Basic {}", api.idp.token("alice", "acme").build()),
        ),
    ));

    let mut bodies: Vec<(&str, Value)> = Vec::new();
    for (label, request) in attempts {
        let response = request.send().await.expect("send");
        assert_eq!(response.status(), 401, "{label}");
        assert!(
            response.headers().contains_key("www-authenticate"),
            "{label}: RFC 6750 challenge"
        );
        trace_id(&response);
        bodies.push((label, without_trace(json_of(response).await)));
    }

    // No oracle: the client cannot tell WHY it was refused.
    let (first_label, first) = &bodies[0];
    for (label, body) in &bodies {
        assert_eq!(
            body, first,
            "'{label}' must be indistinguishable from '{first_label}'"
        );
    }
    assert_eq!(first["code"], "UNAUTHENTICATED");
}

#[tokio::test]
async fn ambiguous_authorization_headers_are_refused() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let good = api.idp.token("alice", "acme").build();

    let two_headers = api
        .request(Method::GET, "/auth/context")
        .header("authorization", format!("Bearer {good}"))
        .header("authorization", format!("Bearer {good}"))
        .send()
        .await
        .expect("send");
    assert_eq!(
        two_headers.status(),
        401,
        "two Authorization headers are ambiguous"
    );

    let spaced = api
        .request(Method::GET, "/auth/context")
        .header("authorization", format!("Bearer {good} extra"))
        .send()
        .await
        .expect("send");
    assert_eq!(spaced.status(), 401);

    let lowercase_scheme = api
        .request(Method::GET, "/auth/context")
        .header("authorization", format!("bearer {good}"))
        .send()
        .await
        .expect("send");
    assert_eq!(
        lowercase_scheme.status(),
        200,
        "the scheme is case-insensitive per RFC 7235"
    );
}

#[tokio::test]
async fn a_revoked_membership_takes_effect_on_the_very_next_request() {
    // Permissions are read from the database on every request, never cached from the token.
    let api = start().await;
    api.tenant("acme", "alice").await;
    let token = api.idp.token("alice", "acme").build();
    let call = || {
        api.request(Method::GET, "/auth/context")
            .bearer_auth(&token)
            .send()
    };
    assert_eq!(call().await.expect("send").status(), 200);

    sqlx::query("DELETE FROM memberships WHERE subject = 'alice'")
        .execute(&api.db.pool)
        .await
        .expect("revoke");

    assert_eq!(
        call().await.expect("send").status(),
        401,
        "same still-valid token, no longer a member"
    );
}

#[tokio::test]
async fn probes_need_no_credentials_and_unknown_routes_use_the_envelope() {
    let api = start().await;

    let health = api
        .request(Method::GET, "/health")
        .send()
        .await
        .expect("send");
    assert_eq!(health.status(), 200);
    assert_eq!(json_of(health).await, json!({"status": "ok"}));
    let ready = api
        .request(Method::GET, "/ready")
        .send()
        .await
        .expect("send");
    assert_eq!(ready.status(), 200);

    let unknown = api
        .request(Method::GET, "/nope")
        .send()
        .await
        .expect("send");
    assert_eq!(unknown.status(), 404);
    trace_id(&unknown);
    let unknown = json_of(unknown).await;
    assert_eq!(unknown["code"], "NOT_FOUND");
    // A route the server does not have says so, so that an SDK newer than the server can tell "this call
    // does not exist here" from "this run does not exist" (feature 006, SPEC §9).
    assert_eq!(unknown["details"]["reason"], "ROUTE_NOT_FOUND", "{unknown}");

    let wrong_method = api
        .request(Method::DELETE, "/auth/context")
        .send()
        .await
        .expect("send");
    assert_eq!(wrong_method.status(), 405);
    let wrong_method = json_of(wrong_method).await;
    assert_eq!(wrong_method["code"], "BAD_REQUEST");
    assert_eq!(wrong_method["details"]["reason"], "METHOD_NOT_ALLOWED", "{wrong_method}");
}

#[tokio::test]
async fn responses_carry_security_headers_and_are_never_cacheable() {
    let api = start().await;
    api.tenant("acme", "alice").await;

    for response in [
        api.get_as("/auth/context", "alice", "acme").await,
        api.request(Method::GET, "/auth/context")
            .send()
            .await
            .expect("send"),
        api.request(Method::GET, "/health")
            .send()
            .await
            .expect("send"),
    ] {
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }
    let _ = now_secs();
}

//! The OIDC provider (JWKS endpoint) misbehaves.

use std::time::Duration;

use reqwest::Method;

use crate::support::api::{Options, json_of, start, start_with};
use crate::support::oidc::Mode;

fn short_cache(auth: &mut sagat_api_http::AuthConfig) {
    auth.jwks_ttl = Duration::from_millis(400);
    auth.jwks_max_stale = Duration::from_millis(800);
    auth.jwks_min_refresh_interval = Duration::ZERO;
}

#[tokio::test]
async fn a_provider_that_is_down_at_startup_fails_closed_and_recovery_is_automatic() {
    let api = start_with(Options {
        auth: Box::new(short_cache),
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    api.idp.set_mode(Mode::Status(500));

    let down = api.get_as("/auth/context", "alice", "acme").await;
    assert_eq!(
        down.status(),
        503,
        "no keys, so nobody can be authenticated"
    );
    assert_eq!(down.headers()["retry-after"], "5");
    let body = json_of(down).await;
    assert_eq!(body["code"], "UNAVAILABLE");
    assert_eq!(body["retryable"], true);

    // Liveness and readiness do not depend on the provider: the pod must not be restarted for it.
    assert_eq!(
        api.request(Method::GET, "/health")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );
    assert_eq!(
        api.request(Method::GET, "/ready")
            .send()
            .await
            .expect("send")
            .status(),
        200
    );

    api.idp.set_mode(Mode::Normal);
    let recovered = api.get_as("/auth/context", "alice", "acme").await;
    assert_eq!(
        recovered.status(),
        200,
        "the next request after the provider returns succeeds"
    );
}

#[tokio::test]
async fn cached_keys_ride_out_a_short_outage_and_then_expire_into_a_fail_closed_503() {
    let api = start_with(Options {
        auth: Box::new(short_cache),
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200,
        "warm the cache"
    );
    api.idp.set_mode(Mode::Status(500));

    // Keys expired (ttl 400 ms) but the stale window (800 ms more) still covers us.
    tokio::time::sleep(Duration::from_millis(550)).await;
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200,
        "stale keys serve"
    );

    // Past ttl + max_stale (1.2 s) the keys are too old to trust: fail closed.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        503
    );

    api.idp.set_mode(Mode::Normal);
    assert_eq!(
        api.get_as("/auth/context", "alice", "acme").await.status(),
        200,
        "and recovers"
    );
}

#[tokio::test]
async fn a_provider_that_answers_badly_never_results_in_access() {
    for (label, mode) in [
        ("redirect (must not be followed)", Mode::Redirect),
        ("oversized body", Mode::Oversized),
        ("not a key set", Mode::Garbage),
        ("404", Mode::Status(404)),
    ] {
        let api = start().await;
        api.tenant("acme", "alice").await;
        api.idp.set_mode(mode);

        let response = api.get_as("/auth/context", "alice", "acme").await;

        assert_eq!(response.status(), 503, "{label}");
        assert_eq!(json_of(response).await["code"], "UNAVAILABLE", "{label}");
    }
}

#[tokio::test]
async fn a_provider_that_hangs_is_cut_off_by_the_client_timeout() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    api.idp.set_mode(Mode::Hang);

    let started = std::time::Instant::now();
    let response = api.get_as("/auth/context", "alice", "acme").await;

    assert_eq!(response.status(), 503);
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "bounded by the 5 s fetch timeout, took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn an_outage_does_not_turn_into_a_stampede_of_provider_requests() {
    let api = start().await; // default: refreshes at most every 30 s (1 s while there are no keys)
    api.tenant("acme", "alice").await;
    api.idp.set_mode(Mode::Status(500));

    let requests: Vec<_> = (0..40)
        .map(|_| {
            let client = api.client.clone();
            let url = format!("{}/auth/context", api.base);
            let token = api.idp.token("alice", "acme").build();
            tokio::spawn(async move {
                client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .expect("send")
                    .status()
            })
        })
        .collect();
    for request in requests {
        assert_eq!(request.await.expect("join"), 503);
    }

    assert!(
        api.idp.jwks_hits() <= 2,
        "40 concurrent requests caused {} provider fetches",
        api.idp.jwks_hits()
    );
}

#[tokio::test]
async fn a_key_revoked_at_the_provider_stops_working_once_the_cache_refreshes() {
    let api = start_with(Options {
        auth: Box::new(short_cache),
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let token = api.idp.token("alice", "acme").build();
    let call = || {
        api.request(Method::GET, "/auth/context")
            .bearer_auth(&token)
            .send()
    };
    assert_eq!(call().await.expect("send").status(), 200);

    api.idp.rotate(false); // the signing key is withdrawn
    tokio::time::sleep(Duration::from_millis(500)).await; // let the cached copy expire

    assert_eq!(
        call().await.expect("send").status(),
        401,
        "tokens signed by the withdrawn key are refused"
    );
}

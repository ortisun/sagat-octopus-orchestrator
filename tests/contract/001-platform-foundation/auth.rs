//! Token verification matrix: one test per defence, all against a fake OIDC provider over HTTP.

use std::sync::Arc;
use std::time::Duration;

use sagat_api_http::auth::{
    AuthError, HttpJwksFetcher, InvalidReason, VerifiedClaims, install_crypto_provider,
};
use sagat_api_http::{AuthConfig, TokenVerifier};
use serde_json::{Value, json};

use crate::support::oidc::{
    AUDIENCE, ISSUER, TestIdp, TestKey, now_secs, tamper_payload, unsigned_token,
};

async fn verifier_with(idp: &TestIdp, tweak: impl FnOnce(&mut AuthConfig)) -> TokenVerifier {
    install_crypto_provider();
    let mut config = AuthConfig::new(ISSUER, AUDIENCE, idp.jwks_url());
    config.allow_insecure_jwks_http = true;
    tweak(&mut config);
    TokenVerifier::new(
        Arc::new(HttpJwksFetcher::new(&config).expect("fetcher")),
        &config,
    )
}

async fn verifier(idp: &TestIdp) -> TokenVerifier {
    verifier_with(idp, |_| {}).await
}

fn invalid(reason: InvalidReason) -> Result<VerifiedClaims, AuthError> {
    Err(AuthError::Invalid(reason))
}

#[tokio::test]
async fn a_valid_token_yields_the_subject_and_the_tenant_claim() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let claims = v.verify(&idp.token("alice", "acme").build()).await;

    assert_eq!(
        claims,
        Ok(VerifiedClaims {
            subject: "alice".into(),
            tenant: "acme".into()
        })
    );
}

#[tokio::test]
async fn the_tenant_claim_name_is_configurable() {
    let idp = TestIdp::start().await;
    let v = verifier_with(&idp, |c| {
        c.tenant_claim = "https://example.test/tenant".into()
    })
    .await;

    let default_name = idp.token("alice", "acme").build();
    assert_eq!(
        v.verify(&default_name).await,
        invalid(InvalidReason::TenantClaim)
    );

    let namespaced = idp
        .token("alice", "x")
        .without("tenant")
        .claim("https://example.test/tenant", json!("acme"))
        .build();
    assert_eq!(
        v.verify(&namespaced).await.map(|c| c.tenant),
        Ok("acme".into())
    );
}

// ------------------------------------------------------------------------------- time

#[tokio::test]
async fn expired_tokens_are_refused_but_clock_skew_within_the_leeway_is_tolerated() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let long_expired = idp.token("alice", "acme").issued(600, 100).build();
    assert_eq!(v.verify(&long_expired).await, Err(AuthError::Expired));

    // Expired 10 s ago: inside the 30 s leeway.
    let just_expired = idp.token("alice", "acme").issued(400, 390).build();
    assert!(v.verify(&just_expired).await.is_ok());
}

#[tokio::test]
async fn tokens_not_yet_valid_are_refused() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;
    let future = json!(now_secs() + 600);

    let nbf = idp
        .token("alice", "acme")
        .claim("nbf", future.clone())
        .build();
    assert_eq!(v.verify(&nbf).await, invalid(InvalidReason::NotYetValid));

    let iat = idp
        .token("alice", "acme")
        .claim("iat", future.clone())
        .claim("exp", json!(now_secs() + 900))
        .build();
    assert_eq!(v.verify(&iat).await, invalid(InvalidReason::NotYetValid));
}

#[tokio::test]
async fn token_lifetime_is_bounded_to_limit_the_revocation_window() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let day_long = idp.token("alice", "acme").issued(0, 86_400).build();
    assert_eq!(v.verify(&day_long).await, invalid(InvalidReason::Lifetime));

    let hour_long = idp.token("alice", "acme").issued(0, 3_600).build();
    assert!(
        v.verify(&hour_long).await.is_ok(),
        "exactly the limit is allowed"
    );
}

// ----------------------------------------------------------------------------- claims

#[tokio::test]
async fn issuer_and_audience_must_match_exactly() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let wrong_iss = idp
        .token("a", "acme")
        .claim("iss", json!("https://evil.example/"))
        .build();
    assert_eq!(v.verify(&wrong_iss).await, invalid(InvalidReason::Issuer));

    let wrong_aud = idp
        .token("a", "acme")
        .claim("aud", json!("another-api"))
        .build();
    assert_eq!(v.verify(&wrong_aud).await, invalid(InvalidReason::Audience));

    let aud_list_ok = idp
        .token("a", "acme")
        .claim("aud", json!(["other", AUDIENCE]))
        .build();
    assert!(
        v.verify(&aud_list_ok).await.is_ok(),
        "an audience list containing ours is fine"
    );

    let aud_list_bad = idp
        .token("a", "acme")
        .claim("aud", json!(["other", "another"]))
        .build();
    assert_eq!(
        v.verify(&aud_list_bad).await,
        invalid(InvalidReason::Audience)
    );
}

#[tokio::test]
async fn every_security_relevant_claim_is_required() {
    // The JWT library validates iss/aud/sub only when present; the verifier must require them.
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    for missing in ["iss", "aud", "sub", "exp", "iat"] {
        let token = idp.token("alice", "acme").without(missing).build();
        assert_eq!(
            v.verify(&token).await,
            invalid(InvalidReason::MissingClaim),
            "missing {missing}"
        );
    }
    let no_tenant = idp.token("alice", "acme").without("tenant").build();
    assert_eq!(
        v.verify(&no_tenant).await,
        invalid(InvalidReason::TenantClaim)
    );
}

#[tokio::test]
async fn the_tenant_claim_must_be_a_short_non_empty_string() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let bad: [Value; 6] = [
        json!(""),
        json!(42),
        json!(["acme"]),
        json!({"slug": "acme"}),
        json!(null),
        json!("t".repeat(64)),
    ];
    for value in bad {
        let token = idp
            .token("alice", "acme")
            .claim("tenant", value.clone())
            .build();
        assert_eq!(
            v.verify(&token).await,
            invalid(InvalidReason::TenantClaim),
            "{value}"
        );
    }
}

#[tokio::test]
async fn an_empty_or_oversized_subject_is_refused() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    for subject in [String::new(), "s".repeat(256)] {
        let token = idp.token("x", "acme").claim("sub", json!(subject)).build();
        assert!(v.verify(&token).await.is_err());
    }
    let longest = idp.token(&"s".repeat(255), "acme").build();
    assert!(v.verify(&longest).await.is_ok());
}

// -------------------------------------------------------------------------- signature

#[tokio::test]
async fn hmac_tokens_signed_with_the_public_key_are_refused() {
    // Classic key-confusion attack: use the published public key as an HMAC secret.
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;
    let public = idp.key.lock().expect("key").public_bytes();

    let forged = idp.token("mallory", "acme").hmac_with(public).build();

    assert_eq!(v.verify(&forged).await, invalid(InvalidReason::Algorithm));
}

#[tokio::test]
async fn unsigned_tokens_are_refused() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;
    let kid = idp.key.lock().expect("key").kid.clone();
    let claims = json!({"iss": ISSUER, "aud": AUDIENCE, "sub": "mallory", "tenant": "acme",
                        "iat": now_secs(), "exp": now_secs() + 300});

    let result = v.verify(&unsigned_token(&kid, &claims)).await;

    assert!(
        matches!(
            result,
            Err(AuthError::Malformed | AuthError::Invalid(InvalidReason::Algorithm))
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_token_signed_by_another_key_is_refused_even_with_a_known_kid() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;
    let attacker = TestKey::generate();

    // Same `kid` as the real key, signed by the attacker's key material.
    let real_kid = idp.key.lock().expect("key").kid.clone();
    let forged = idp
        .token("mallory", "acme")
        .signed_with(attacker.with_kid(&real_kid))
        .build();

    assert_eq!(v.verify(&forged).await, invalid(InvalidReason::Signature));
}

#[tokio::test]
async fn editing_the_payload_of_a_signed_token_breaks_the_signature() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;
    let token = idp.token("alice", "acme").build();

    // Move alice to another tenant without re-signing.
    let tampered = tamper_payload(&token, |p| p["tenant"] = json!("globex"));

    assert_eq!(v.verify(&tampered).await, invalid(InvalidReason::Signature));
}

#[tokio::test]
async fn key_ids_are_required_and_must_exist() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let none = idp.token("a", "acme").kid(None).build();
    assert_eq!(v.verify(&none).await, invalid(InvalidReason::MissingKeyId));

    let empty = idp.token("a", "acme").kid(Some("")).build();
    assert_eq!(v.verify(&empty).await, invalid(InvalidReason::MissingKeyId));

    let huge = idp.token("a", "acme").kid(Some(&"k".repeat(129))).build();
    assert_eq!(v.verify(&huge).await, invalid(InvalidReason::MissingKeyId));

    let unknown = idp.token("a", "acme").kid(Some("no-such-key")).build();
    assert_eq!(v.verify(&unknown).await, invalid(InvalidReason::UnknownKey));
}

#[tokio::test]
async fn garbage_and_oversized_tokens_are_malformed() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    for token in [
        "",
        "abc",
        "a.b",
        "a.b.c",
        "....",
        "Bearer x",
        "\u{0}\u{0}\u{0}",
    ] {
        assert_eq!(
            v.verify(token).await,
            Err(AuthError::Malformed),
            "{token:?}"
        );
    }
    assert_eq!(
        v.verify(&"a".repeat(9 * 1024)).await,
        Err(AuthError::Malformed)
    );
}

// ------------------------------------------------------------------------ key handling

#[tokio::test]
async fn keys_that_cannot_verify_access_tokens_are_never_used() {
    let idp = TestIdp::start().await;
    let real = idp.key.lock().expect("key").clone();
    let mut encryption_only = real.public_jwk();
    encryption_only["use"] = json!("enc");
    let mut wrong_alg = real.public_jwk();
    wrong_alg["alg"] = json!("RS256");
    let mut without_kid = real.public_jwk();
    without_kid.as_object_mut().expect("obj").remove("kid");
    // The set must contain one genuinely usable key (of someone else), otherwise it would be a
    // useless answer that the cache refuses to adopt at all.
    let unrelated = TestKey::generate();
    idp.publish(vec![
        json!({"kty": "oct", "kid": "hmac-1", "k": "c2VjcmV0", "alg": "HS256"}),
        encryption_only,
        wrong_alg,
        without_kid,
        unrelated.public_jwk(),
    ]);
    let v = verifier(&idp).await;

    // Even a token correctly signed by the real key finds no usable key published under its kid.
    let token = idp.token("alice", "acme").build();
    assert_eq!(v.verify(&token).await, invalid(InvalidReason::UnknownKey));
}

#[tokio::test]
async fn a_rotated_key_is_picked_up_without_a_restart() {
    let idp = TestIdp::start().await;
    let v = verifier_with(&idp, |c| c.jwks_min_refresh_interval = Duration::ZERO).await;
    assert!(v.verify(&idp.token("alice", "acme").build()).await.is_ok());

    idp.rotate(true);

    let with_new_key = idp.token("alice", "acme").build();
    assert!(
        v.verify(&with_new_key).await.is_ok(),
        "unknown kid triggers a refresh"
    );
}

#[tokio::test]
async fn unknown_key_ids_cannot_make_the_api_hammer_the_provider() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await; // default: at most one refresh per 30 s
    assert!(v.verify(&idp.token("alice", "acme").build()).await.is_ok());
    let hits_after_warmup = idp.jwks_hits();

    for i in 0..50 {
        let token = idp
            .token("mallory", "acme")
            .kid(Some(&format!("random-{i}")))
            .build();
        let _ = v.verify(&token).await;
    }

    // One forced refresh at most (the first unknown kid); the other 49 never reach the network.
    assert!(
        idp.jwks_hits() <= hits_after_warmup + 1,
        "{} fetches for 50 bogus kids",
        idp.jwks_hits() - hits_after_warmup
    );
}

#[tokio::test]
async fn the_jwks_must_be_fetched_over_https_unless_explicitly_relaxed() {
    let config = AuthConfig::new(ISSUER, AUDIENCE, "http://idp.example.test/jwks.json");
    assert!(
        HttpJwksFetcher::new(&config).is_err(),
        "plain http is refused by default"
    );

    let https = AuthConfig::new(ISSUER, AUDIENCE, "https://idp.example.test/jwks.json");
    assert!(HttpJwksFetcher::new(&https).is_ok());

    let ftp = AuthConfig::new(ISSUER, AUDIENCE, "ftp://idp.example.test/jwks.json");
    assert!(HttpJwksFetcher::new(&ftp).is_err());
}

#[tokio::test]
async fn tokens_declaring_critical_header_extensions_are_refused() {
    // RFC 7515 §4.1.11: a verifier that does not implement a `crit` extension must reject the token.
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let token = idp.token("alice", "acme").crit(&["exp"]).build();

    assert_eq!(
        v.verify(&token).await,
        invalid(InvalidReason::CriticalHeader)
    );
}

#[tokio::test]
async fn the_issuer_must_be_a_string_not_a_list_containing_it() {
    let idp = TestIdp::start().await;
    let v = verifier(&idp).await;

    let as_list = idp
        .token("alice", "acme")
        .claim("iss", json!([ISSUER, "https://evil.example/"]))
        .build();
    assert_eq!(v.verify(&as_list).await, invalid(InvalidReason::Issuer));

    let exact = idp.token("alice", "acme").build();
    assert!(v.verify(&exact).await.is_ok());
}

#[tokio::test]
async fn a_required_token_type_keeps_other_kinds_of_token_out() {
    // RFC 9068: access tokens are `at+jwt`; this keeps an ID token (typ JWT) from being replayed as one.
    let idp = TestIdp::start().await;
    let v = verifier_with(&idp, |c| c.required_token_type = Some("at+jwt".into())).await;

    let plain = idp.token("alice", "acme").build(); // typ = JWT
    assert_eq!(v.verify(&plain).await, invalid(InvalidReason::TokenType));
    let none = idp.token("alice", "acme").typ(None).build();
    assert_eq!(v.verify(&none).await, invalid(InvalidReason::TokenType));
    for ok in ["at+jwt", "AT+JWT"] {
        let token = idp.token("alice", "acme").typ(Some(ok)).build();
        assert!(v.verify(&token).await.is_ok(), "{ok}");
    }

    // Off by default.
    let lenient = verifier(&idp).await;
    assert!(lenient.verify(&plain).await.is_ok());
}

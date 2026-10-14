//! Real signatures and HTTP JWKS requests at the webhook identity boundary.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../tests/support/oidc.rs"]
mod oidc;

use sagat_api_http::auth::{
    AuthConfig, HttpJwksFetcher, TokenVerifier, WebhookOidcConfig, WebhookOidcVerifiers,
};
use sagat_application::{error::AppError, events::WebhookIdentityVerifier};
use sagat_domain::identity::Slug;
use serde_json::json;
use std::sync::Arc;

fn config(idp: &oidc::TestIdp) -> WebhookOidcConfig {
    WebhookOidcConfig {
        name: "provider".to_owned(),
        issuer: oidc::ISSUER.to_owned(),
        audience: oidc::AUDIENCE.to_owned(),
        jwks_uri: idp.jwks_url(),
        required_token_type: None,
    }
}

#[tokio::test]
async fn external_subject_needs_no_platform_tenant_but_api_tokens_still_do() {
    let idp = oidc::TestIdp::start().await;
    let key = idp.key.lock().expect("key").clone();
    let token = oidc::TokenBuilder::new(key.clone(), "provider-account", "unused")
        .without("tenant")
        .build();
    let profiles = WebhookOidcVerifiers::new(&[config(&idp)], true).expect("profiles");
    let profile = Slug::parse("provider").expect("profile");
    assert_eq!(
        profiles.verify(&profile, &token).await.expect("verified"),
        "provider-account"
    );
    let mut auth = AuthConfig::new(oidc::ISSUER, oidc::AUDIENCE, idp.jwks_url());
    auth.allow_insecure_jwks_http = true;
    let api = TokenVerifier::new(
        Arc::new(HttpJwksFetcher::new(&auth).expect("fetcher")),
        &auth,
    );
    assert!(
        api.verify(&token).await.is_err(),
        "platform tenant claim remains mandatory"
    );
    for (name, value) in [
        ("iss", json!("https://other.invalid")),
        ("aud", json!("other")),
        ("exp", json!(1)),
    ] {
        let invalid = oidc::TokenBuilder::new(key.clone(), "provider-account", "unused")
            .claim(name, value)
            .build();
        assert!(matches!(
            profiles.verify(&profile, &invalid).await,
            Err(AppError::Unauthenticated)
        ));
    }
    assert!(matches!(
        profiles
            .verify(&Slug::parse("unknown").expect("slug"), &token)
            .await,
        Err(AppError::Unauthenticated)
    ));
}

#[tokio::test]
async fn profiles_isolate_equal_key_ids_and_jwks_failure_is_retryable() {
    let first = oidc::TestIdp::start().await;
    let second = oidc::TestIdp::start().await;
    let first_key = first.key.lock().expect("key").clone();
    let second_key = second
        .key
        .lock()
        .expect("key")
        .clone()
        .with_kid(&first_key.kid);
    second.publish(vec![second_key.public_jwk()]);
    let first_config = config(&first);
    let mut other_config = config(&second);
    other_config.name = "other".to_owned();
    let profiles =
        WebhookOidcVerifiers::new(&[first_config, other_config], true).expect("profiles");
    let token =
        oidc::TokenBuilder::new(first.key.lock().expect("key").clone(), "sender", "unused").build();
    assert!(
        profiles
            .verify(&Slug::parse("provider").expect("slug"), &token)
            .await
            .is_ok()
    );
    assert!(matches!(
        profiles
            .verify(&Slug::parse("other").expect("slug"), &token)
            .await,
        Err(AppError::Unauthenticated)
    ));
    second.set_mode(oidc::Mode::Status(503));
    let unavailable = WebhookOidcVerifiers::new(&[config(&second)], true).expect("profiles");
    assert!(matches!(
        unavailable
            .verify(&Slug::parse("provider").expect("slug"), &token)
            .await,
        Err(AppError::Unavailable)
    ));
    assert!(
        WebhookOidcVerifiers::new(&[config(&first)], false).is_err(),
        "plaintext keys cannot be configured in production"
    );
}

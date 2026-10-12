//! Real HTTPS, JWT verification and inbox admission through the public routes.
use crate::support::{
    api::{Options, start_with},
    oidc::{self, TestIdp, TokenBuilder},
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use sagat_api_http::{HttpTlsConfig, auth::WebhookOidcConfig};
use serde_json::{Value, json};
use sha2::Digest as _;

#[tokio::test]
async fn native_mtls_and_oidc_profiles_share_the_durable_inbox_without_fallback() {
    let ca_key = KeyPair::generate().expect("CA key");
    let mut params = CertificateParams::new(Vec::new()).expect("params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = params.self_signed(&ca_key).expect("CA");
    let issue = || {
        let key = KeyPair::generate().expect("key");
        let params = CertificateParams::new(vec!["localhost".to_owned()]).expect("params");
        (params.signed_by(&key, &ca, &ca_key).expect("leaf"), key)
    };
    let (server, server_key) = issue();
    let (sender, sender_key) = issue();
    let fingerprint: String = sha2::Sha256::digest(sender.der().as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let provider = TestIdp::start().await;
    let api = start_with(Options {
        http_tls: Some(
            HttpTlsConfig::from_pem(
                server.pem().as_bytes(),
                server_key.serialize_pem().as_bytes(),
                ca.pem().as_bytes(),
            )
            .expect("TLS"),
        ),
        http_tls_ca: Some(ca.pem().into_bytes()),
        webhook_oidc_profiles: vec![WebhookOidcConfig {
            name: "provider".to_owned(),
            issuer: oidc::ISSUER.to_owned(),
            audience: oidc::AUDIENCE.to_owned(),
            jwks_uri: provider.jwks_url(),
            required_token_type: None,
        }],
        ..Options::default()
    })
    .await;
    let api_key = api.idp.key.lock().expect("key").clone();
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let create = |slug: &str, authentication: Value| {
        api.client.post(format!("{}/namespaces/{ns}/event-endpoints", api.base))
        .bearer_auth(TokenBuilder::new(api_key.clone(), "alice", "acme").build())
        .json(&json!({"slug":slug, "authentication":authentication, "defaultEventType":"invoice.paid"}))
    };
    let mtls = create(
        "mutual-tls",
        json!({"kind":"MTLS", "certificateSha256":fingerprint}),
    )
    .send()
    .await
    .expect("create");
    assert_eq!(mtls.status(), 201, "{}", mtls.text().await.expect("body"));
    // Resolve through list so the status assertion above consumes only failure text.
    let list: Value = api
        .client
        .get(format!("{}/namespaces/{ns}/event-endpoints", api.base))
        .bearer_auth(TokenBuilder::new(api_key.clone(), "alice", "acme").build())
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("JSON");
    let mtls_id = list["items"][0]["id"].as_str().expect("id");
    let oidc = create(
        "openid",
        json!({"kind":"OIDC","profile":"provider","subject":"external-sender"}),
    )
    .send()
    .await
    .expect("create");
    assert_eq!(oidc.status(), 201);
    let oidc: Value = oidc.json().await.expect("JSON");
    let cert_client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(ca.pem().as_bytes()).expect("root"))
        .identity(
            reqwest::Identity::from_pem(
                format!("{}{}", sender.pem(), sender_key.serialize_pem()).as_bytes(),
            )
            .expect("identity"),
        )
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    let (other_sender, other_key) = issue();
    let other_client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(ca.pem().as_bytes()).expect("root"))
        .identity(
            reqwest::Identity::from_pem(
                format!("{}{}", other_sender.pem(), other_key.serialize_pem()).as_bytes(),
            )
            .expect("identity"),
        )
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    let request = |client: &reqwest::Client, id: &str| {
        client
            .post(format!("{}/webhooks/{id}", api.base))
            .header(
                "x-octopus-timestamp",
                time::OffsetDateTime::now_utc().unix_timestamp().to_string(),
            )
            .header("x-octopus-event-id", "provider-event-1")
    };
    assert_eq!(
        request(&api.client, mtls_id)
            .header("x-forwarded-client-cert", &fingerprint)
            .json(&json!({"n":1}))
            .send()
            .await
            .expect("spoof")
            .status(),
        401
    );
    assert_eq!(
        request(&other_client, mtls_id)
            .json(&json!({"n":1}))
            .send()
            .await
            .expect("trusted but unpinned")
            .status(),
        401
    );
    for _ in 0..2 {
        assert_eq!(
            request(&cert_client, mtls_id)
                .json(&json!({"n":1}))
                .send()
                .await
                .expect("TLS webhook")
                .status(),
            202
        );
    }
    assert_eq!(
        request(&cert_client, mtls_id)
            .json(&json!({"n":2}))
            .send()
            .await
            .expect("reuse")
            .status(),
        409
    );
    let oidc_id = oidc["id"].as_str().expect("id");
    let key = provider.key.lock().expect("key").clone();
    let token = TokenBuilder::new(key.clone(), "external-sender", "unused")
        .without("tenant")
        .build();
    assert_eq!(
        request(&api.client, oidc_id)
            .bearer_auth(&token)
            .json(&json!({"n":1}))
            .send()
            .await
            .expect("OIDC webhook")
            .status(),
        202
    );
    let wrong = TokenBuilder::new(key, "other-sender", "unused").build();
    assert_eq!(
        request(&cert_client, oidc_id)
            .bearer_auth(wrong)
            .json(&json!({"n":1}))
            .send()
            .await
            .expect("wrong subject")
            .status(),
        401,
        "a valid TLS identity cannot bypass a failed OIDC profile"
    );
    let foreign = api
        .client
        .get(format!(
            "{}/namespaces/{ns}/event-endpoints/{mtls_id}",
            api.base
        ))
        .bearer_auth(TokenBuilder::new(api_key.clone(), "bob", "globex").build())
        .send()
        .await
        .expect("foreign");
    assert_eq!(foreign.status(), 404);
}

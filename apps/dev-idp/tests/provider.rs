//! The development identity provider: what it refuses to be, what it issues, and what it refuses to
//! issue to.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use sagat_dev_idp::{Config, ConfigError, DevKey, KeyError, router};
use serde_json::{Value, json};

fn config(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let env: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Config::from_lookup(&|name| env.get(name).cloned())
}

#[test]
fn it_only_runs_in_development() {
    for env in [
        None,
        Some(""),
        Some("prod"),
        Some("staging"),
        Some("DEV"),
        Some("dev2"),
    ] {
        let pairs: Vec<(&str, &str)> = env.map(|e| ("SAGAT_ENV", e)).into_iter().collect();
        assert_eq!(config(&pairs), Err(ConfigError::NotDevelopment), "{env:?}");
    }
    let ok = config(&[("SAGAT_ENV", " dev ")]).expect("dev, trimmed");
    assert_eq!(
        ok.addr,
        "127.0.0.1:9100".parse::<SocketAddr>().expect("addr")
    );
    assert_eq!(ok.issuer, "http://127.0.0.1:9100/");
    assert_eq!(ok.audience, "sagat-octopus-orchestrator");
    assert_eq!(ok.tenant_claim, "tenant");
}

#[test]
fn it_listens_beyond_loopback_only_when_told_that_is_meant() {
    let open = [("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_ADDR", "0.0.0.0:9100")];
    assert!(matches!(config(&open), Err(ConfigError::Exposed(_))));
    let told = [
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ADDR", "0.0.0.0:9100"),
        ("SAGAT_DEV_IDP_EXPOSED", "true"),
    ];
    assert!(config(&told).is_ok());
    // "yes" is not "true": it is a switch, not a hope.
    let vague = [
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ADDR", "0.0.0.0:9100"),
        ("SAGAT_DEV_IDP_EXPOSED", "yes"),
    ];
    assert!(matches!(config(&vague), Err(ConfigError::Exposed(_))));
    for loopback in ["127.0.0.1:1", "[::1]:1", "[::ffff:127.0.0.1]:1"] {
        assert!(
            config(&[("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_ADDR", loopback)]).is_ok(),
            "{loopback}"
        );
    }
    assert!(matches!(
        config(&[("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_ADDR", "nowhere")]),
        Err(ConfigError::Invalid {
            name: "SAGAT_DEV_IDP_ADDR",
            ..
        })
    ));
}

#[test]
fn an_issuer_or_a_claim_that_could_confuse_the_platform_is_refused() {
    for issuer in [
        "ftp://idp/",
        "http://user:pw@idp/",
        "idp.example",
        "http://id p/",
    ] {
        assert!(
            matches!(
                config(&[("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_ISSUER", issuer)]),
                Err(ConfigError::Invalid {
                    name: "SAGAT_DEV_IDP_ISSUER",
                    ..
                })
            ),
            "{issuer}"
        );
    }
    for claim in ["sub", "exp", "iss", "aud"] {
        assert!(
            matches!(
                config(&[("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_TENANT_CLAIM", claim)]),
                Err(ConfigError::Invalid {
                    name: "SAGAT_DEV_IDP_TENANT_CLAIM",
                    ..
                })
            ),
            "{claim}"
        );
    }
}

#[test]
fn other_host_names_are_added_only_when_they_are_names() {
    let ok = config(&[
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_HOSTS", "127.0.0.1:18090, localhost:18090"),
    ])
    .expect("hosts");
    assert_eq!(ok.extra_hosts, vec!["127.0.0.1:18090", "localhost:18090"]);
    assert!(ok.hosts().contains(&"localhost:18090".to_owned()));
    for bad in [
        "localhost",
        "http://localhost:1",
        "a b:1",
        "host:99999",
        ":80",
    ] {
        assert!(
            matches!(
                config(&[("SAGAT_ENV", "dev"), ("SAGAT_DEV_IDP_HOSTS", bad)]),
                Err(ConfigError::Invalid {
                    name: "SAGAT_DEV_IDP_HOSTS",
                    ..
                })
            ),
            "{bad}"
        );
    }
}

/// The provider on a port of its own, and the base URL to reach it by.
async fn serve(extra: &[(&str, &str)]) -> (String, Config) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let addr = format!("127.0.0.1:{port}");
    let issuer = format!("http://{addr}/");
    let mut pairs = vec![
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ADDR", addr.as_str()),
        ("SAGAT_DEV_IDP_ISSUER", issuer.as_str()),
    ];
    pairs.extend_from_slice(extra);
    let config = config(&pairs).expect("config");
    let app = router(config.clone(), DevKey::generate().expect("key"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), config)
}

fn http() -> reqwest::Client {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

async fn token(base: &str, body: &Value) -> reqwest::Response {
    http()
        .post(format!("{base}/token"))
        .json(body)
        .send()
        .await
        .expect("send")
}

#[tokio::test]
async fn a_token_is_signed_by_the_key_it_publishes_and_says_what_the_platform_reads() {
    let (base, config) = serve(&[]).await;
    let jwks: Value = http()
        .get(format!("{base}/.well-known/jwks.json"))
        .send()
        .await
        .expect("jwks")
        .json()
        .await
        .expect("json");
    let key = &jwks["keys"][0];
    assert_eq!(
        (
            key["kty"].as_str(),
            key["alg"].as_str(),
            key["use"].as_str()
        ),
        (Some("EC"), Some("ES256"), Some("sig"))
    );

    let response = token(
        &base,
        &json!({ "subject": "alice", "tenant": "acme", "ttlSeconds": 120 }),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value = response.json().await.expect("json");
    assert_eq!(
        (body["token_type"].as_str(), body["expires_in"].as_u64()),
        (Some("Bearer"), Some(120))
    );
    let jwt = body["access_token"].as_str().expect("token");

    let header = jsonwebtoken::decode_header(jwt).expect("header");
    assert_eq!(header.alg, Algorithm::ES256);
    assert_eq!(header.kid.as_deref(), key["kid"].as_str());
    let decoding = DecodingKey::from_ec_components(
        key["x"].as_str().expect("x"),
        key["y"].as_str().expect("y"),
    )
    .expect("key");
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[config.audience.as_str()]);
    validation.set_issuer(&[config.issuer.as_str()]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims: Value = jsonwebtoken::decode(jwt, &decoding, &validation)
        .expect("verifies with the published key")
        .claims;
    assert_eq!(claims["sub"], "alice");
    assert_eq!(claims["tenant"], "acme");
    assert_eq!(
        claims["exp"].as_u64().expect("exp") - claims["iat"].as_u64().expect("iat"),
        120
    );

    // By default the most the platform takes.
    let long: Value = token(&base, &json!({ "subject": "alice", "tenant": "acme" }))
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(long["expires_in"], 3600);
}

#[tokio::test]
async fn the_tenant_claim_and_the_audience_follow_the_configuration() {
    let (base, _) = serve(&[
        ("SAGAT_DEV_IDP_TENANT_CLAIM", "org"),
        ("SAGAT_DEV_IDP_AUDIENCE", "my-api"),
    ])
    .await;
    let body: Value = token(&base, &json!({ "subject": "bob", "tenant": "globex" }))
        .await
        .json()
        .await
        .expect("json");
    let jwt = body["access_token"].as_str().expect("token");
    let payload = jwt.split('.').nth(1).expect("payload");
    use base64::Engine;
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("b64"),
    )
    .expect("json");
    assert_eq!(
        (claims["org"].as_str(), claims["aud"].as_str()),
        (Some("globex"), Some("my-api"))
    );
    assert!(claims.get("tenant").is_none());
}

#[tokio::test]
async fn what_it_will_not_sign_is_refused_with_the_reason() {
    let (base, _) = serve(&[]).await;
    for (body, error) in [
        (
            json!({ "subject": "", "tenant": "acme" }),
            "invalid_subject",
        ),
        (
            json!({ "subject": "system:operator", "tenant": "acme" }),
            "invalid_subject",
        ),
        (
            json!({ "subject": "alice", "tenant": "Not A Slug" }),
            "invalid_tenant",
        ),
        (
            json!({ "subject": "alice", "tenant": "" }),
            "invalid_tenant",
        ),
        (
            json!({ "subject": "alice", "tenant": "acme", "ttlSeconds": 0 }),
            "invalid_ttl",
        ),
        (
            json!({ "subject": "alice", "tenant": "acme", "ttlSeconds": 3601 }),
            "invalid_ttl",
        ),
        (
            json!({ "subject": "alice", "tenant": "acme", "role": "admin" }),
            "invalid_request",
        ),
        (json!({ "subject": "alice" }), "invalid_request"),
    ] {
        let response = token(&base, &body).await;
        assert_eq!(response.status(), 400, "{body}");
        let answer: Value = response.json().await.expect("json");
        assert_eq!(answer["error"], error, "{body}");
        assert!(answer.get("access_token").is_none());
    }
}

#[tokio::test]
async fn a_request_a_web_page_could_make_is_refused() {
    let (base, _) = serve(&[]).await;
    // A form or a text body is what a page of another origin can send without asking first.
    for (content_type, body) in [
        (
            "application/x-www-form-urlencoded",
            "subject=alice&tenant=acme",
        ),
        ("text/plain", r#"{"subject":"alice","tenant":"acme"}"#),
    ] {
        let response = http()
            .post(format!("{base}/token"))
            .header("content-type", content_type)
            .body(body)
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 415, "{content_type}");
    }
    let none = http()
        .post(format!("{base}/token"))
        .body("{}")
        .send()
        .await
        .expect("send");
    assert_eq!(none.status(), 415);

    // A name that resolves here but is not one the provider is known by (DNS rebinding).
    for path in [
        "/token",
        "/.well-known/jwks.json",
        "/.well-known/openid-configuration",
    ] {
        let request = if path == "/token" {
            http()
                .post(format!("{base}{path}"))
                .json(&json!({ "subject": "alice", "tenant": "acme" }))
        } else {
            http().get(format!("{base}{path}"))
        };
        let response = request
            .header("host", "evil.example")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 403, "{path}");
    }

    // Large bodies are not read.
    let big = "x".repeat(10_000);
    let response = token(&base, &json!({ "subject": big, "tenant": "acme" })).await;
    assert_eq!(response.status(), 413);
    // A token is not obtained by GET, and a preflight is not answered with permission.
    let get = http()
        .get(format!("{base}/token"))
        .send()
        .await
        .expect("send");
    assert_eq!(get.status(), 405);
    let preflight = http()
        .request(reqwest::Method::OPTIONS, format!("{base}/token"))
        .header("origin", "https://evil.example")
        .header("access-control-request-method", "POST")
        .send()
        .await
        .expect("send");
    assert!(
        preflight
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[tokio::test]
async fn discovery_says_where_the_keys_and_the_tokens_are() {
    let (base, config) = serve(&[]).await;
    let doc: Value = http()
        .get(format!("{base}/.well-known/openid-configuration"))
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    assert_eq!(doc["issuer"], config.issuer.as_str());
    assert_eq!(
        doc["jwks_uri"],
        format!("{}.well-known/jwks.json", config.issuer)
    );
    assert_eq!(doc["token_endpoint"], format!("{}token", config.issuer));
    assert_eq!(
        http()
            .get(format!("{base}/livez"))
            .send()
            .await
            .expect("send")
            .status(),
        200
    );
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sagat-dev-idp-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir.join("key")
}

#[test]
fn a_key_kept_in_a_file_is_the_same_key_next_time_and_only_its_owner_reads_it() {
    let path = temp_file("kept");
    let first = DevKey::load_or_create(&path).expect("created");
    let second = DevKey::load_or_create(&path).expect("loaded");
    assert_eq!(first.kid(), second.kid());
    assert_eq!(first.public_jwk(), second.public_jwk());
    assert_ne!(first.kid(), DevKey::generate().expect("another").kid());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // Made readable by others since: refused, not used.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(matches!(
            DevKey::load_or_create(&path),
            Err(KeyError::Open(_))
        ));
    }
}

#[test]
fn a_file_that_is_not_a_key_is_refused_and_left_alone() {
    let path = temp_file("corrupt");
    std::fs::write(&path, "this is not a key").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    assert!(matches!(
        DevKey::load_or_create(&path),
        Err(KeyError::Corrupt(_))
    ));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "this is not a key"
    );
}

/// A request written by hand, so that what a client library would never send can be sent: the status
/// line of the answer.
async fn raw(base: &str, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = base.trim_start_matches("http://");
    let mut socket = tokio::net::TcpStream::connect(addr).await.expect("connect");
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut answer = Vec::new();
    let _ = socket.read_to_end(&mut answer).await;
    String::from_utf8_lossy(&answer)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn only_a_host_the_provider_is_known_by_is_answered_whatever_the_request_looks_like() {
    let (base, _) = serve(&[]).await;
    let port = base.rsplit(':').next().expect("port");
    let ok =
        |host: &str| format!("GET /livez HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    // /livez answers any request; the keys and the token are the ones behind the rule.
    let keys = |lines: &str| {
        format!("GET /.well-known/jwks.json HTTP/1.1\r\n{lines}Connection: close\r\n\r\n")
    };
    assert!(
        raw(&base, &keys(&format!("Host: 127.0.0.1:{port}\r\n")))
            .await
            .contains(" 200")
    );
    for host in [
        format!("localhost.evil.com:{port}"),
        format!("127.0.0.1:{}", port.parse::<u16>().expect("port") ^ 1),
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{port}.evil.com"),
        format!("evil.example:{port}"),
    ] {
        let status = raw(&base, &keys(&format!("Host: {host}\r\n"))).await;
        assert!(status.contains(" 403"), "{host}: {status}");
    }
    // No Host at all, and two of them: a proxy and a client could be saying different things.
    assert!(
        raw(&base, &keys("")).await.contains(" 403")
            || raw(&base, &keys("")).await.contains(" 400")
    );
    let two = keys(&format!("Host: 127.0.0.1:{port}\r\nHost: evil.example\r\n"));
    assert!(raw(&base, &two).await.contains(" 403"));
    let two_the_other_way = keys(&format!("Host: evil.example\r\nHost: 127.0.0.1:{port}\r\n"));
    assert!(raw(&base, &two_the_other_way).await.contains(" 403"));
    // A request line in absolute form names an authority of its own: it is held to the same rule.
    let absolute = format!(
        "GET http://evil.example/.well-known/jwks.json HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    assert!(raw(&base, &absolute).await.contains(" 403"));
    let absolute_known = format!(
        "GET http://127.0.0.1:{port}/.well-known/jwks.json HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    assert!(raw(&base, &absolute_known).await.contains(" 200"));
    let _ = ok;
}

#[test]
fn the_issuers_own_name_is_known_as_a_client_writes_it() {
    let with_port = config(&[
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ISSUER", "http://IDP.Internal:80/"),
    ])
    .expect("config");
    let hosts = with_port.hosts();
    assert!(hosts.contains(&"idp.internal".to_owned()), "{hosts:?}");
    assert!(hosts.contains(&"idp.internal:80".to_owned()), "{hosts:?}");
    let secure = config(&[
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ISSUER", "https://idp.internal:443/"),
    ])
    .expect("config");
    assert!(secure.hosts().contains(&"idp.internal".to_owned()));
    let other_port = config(&[
        ("SAGAT_ENV", "dev"),
        ("SAGAT_DEV_IDP_ISSUER", "http://idp.internal:9100/"),
    ])
    .expect("config");
    assert!(!other_port.hosts().contains(&"idp.internal".to_owned()));
}

#[cfg(unix)]
#[test]
fn a_key_is_not_taken_through_a_link_or_from_a_directory_others_can_write() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_file("link");
    let dir = path.parent().expect("dir").to_owned();
    // A link, even to a good key, is refused.
    let good = DevKey::load_or_create(&dir.join("real")).expect("real key");
    std::os::unix::fs::symlink(dir.join("real"), &path).expect("link");
    assert!(matches!(
        DevKey::load_or_create(&path),
        Err(KeyError::Link(_))
    ));
    // A directory others can write to: refused, for reading and for creating.
    let shared = dir.join("shared");
    std::fs::create_dir_all(&shared).expect("dir");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    assert!(matches!(
        DevKey::load_or_create(&shared.join("key")),
        Err(KeyError::Shared(_))
    ));
    assert!(!shared.join("key").exists());
    // With the sticky bit, as /tmp has, others cannot remove what is not theirs: accepted.
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).expect("chmod");
    DevKey::load_or_create(&shared.join("key")).expect("sticky directory");
    let _ = good;
}

#[test]
fn a_key_file_that_is_empty_says_to_delete_it_and_two_starts_at_once_share_one_key() {
    let path = temp_file("race");
    let dir = path.parent().expect("dir").to_owned();
    // What a crash in the middle of an older version's write left behind.
    let empty = dir.join("empty");
    std::fs::write(&empty, "").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    let error = DevKey::load_or_create(&empty)
        .err()
        .expect("refused")
        .to_string();
    assert!(error.contains("delete it"), "{error}");

    // Eight starts at once: one key is made and every one of them has it, and no half-written file
    // is left under any name.
    let kids: Vec<String> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || DevKey::load_or_create(&path).expect("key").kid().to_owned())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().expect("thread"))
        .collect();
    assert!(kids.windows(2).all(|w| w[0] == w[1]), "{kids:?}");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

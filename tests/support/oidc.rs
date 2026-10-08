//! A fake OIDC provider for tests: a real HTTP server publishing a JWKS (with failure modes) and a
//! token builder that signs genuine ES256 JWTs. The system under test talks to it over HTTP exactly
//! as it would to a real provider.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;
use sagat_domain::identity::TenantId;
use serde_json::{Map, Value, json};

pub const ISSUER: &str = "https://idp.test.invalid/";
pub const AUDIENCE: &str = "sagat-octopus-orchestrator";

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// An ES256 signing key with the identifiers needed to publish and use it.
#[derive(Clone)]
pub struct TestKey {
    pub kid: String,
    signing: SigningKey,
}

impl TestKey {
    pub fn generate() -> Self {
        // 32 random bytes from two v4 UUIDs; a valid P-256 scalar with overwhelming probability.
        let mut seed = [0u8; 32];
        seed[..16].copy_from_slice(TenantId::generate().as_uuid().as_bytes());
        seed[16..].copy_from_slice(TenantId::generate().as_uuid().as_bytes());
        let signing = SigningKey::from_slice(&seed).expect("valid scalar");
        Self {
            kid: format!("key-{}", TenantId::generate().as_uuid().simple()),
            signing,
        }
    }

    /// The same key material announced under a different `kid` (to impersonate another key).
    pub fn with_kid(mut self, kid: &str) -> Self {
        self.kid = kid.to_owned();
        self
    }

    pub fn public_jwk(&self) -> Value {
        let point = self.signing.verifying_key().to_encoded_point(false);
        json!({
            "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": self.kid,
            "x": URL_SAFE_NO_PAD.encode(point.x().expect("x")),
            "y": URL_SAFE_NO_PAD.encode(point.y().expect("y")),
        })
    }

    fn encoding_key(&self) -> EncodingKey {
        let der = self.signing.to_pkcs8_der().expect("pkcs8");
        EncodingKey::from_ec_der(der.as_bytes())
    }

    /// Raw public key bytes (SEC1 uncompressed), used to attempt an HMAC key-confusion attack.
    pub fn public_bytes(&self) -> Vec<u8> {
        self.signing
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }
}

/// How the fake provider answers `GET /jwks.json`.
#[derive(Clone)]
pub enum Mode {
    Normal,
    Status(u16),
    /// Respond with a redirect to another path (the client must not follow it).
    Redirect,
    /// Never answer within the client's timeout.
    Hang,
    /// A body larger than the client accepts.
    Oversized,
    /// Not a JWKS.
    Garbage,
}

struct IdpState {
    keys: Vec<Value>,
    mode: Mode,
}

pub struct TestIdp {
    state: Arc<Mutex<IdpState>>,
    hits: Arc<AtomicUsize>,
    addr: std::net::SocketAddr,
    pub key: Mutex<TestKey>,
}

#[derive(Clone)]
struct ServerState {
    state: Arc<Mutex<IdpState>>,
    hits: Arc<AtomicUsize>,
}

async fn jwks(State(server): State<ServerState>) -> Response {
    server.hits.fetch_add(1, Ordering::SeqCst);
    let (mode, keys) = {
        let guard = server.state.lock().expect("idp state");
        (guard.mode.clone(), guard.keys.clone())
    };
    match mode {
        Mode::Normal => axum::Json(json!({ "keys": keys })).into_response(),
        Mode::Status(code) => StatusCode::from_u16(code).expect("status").into_response(),
        Mode::Redirect => {
            (StatusCode::FOUND, [(header::LOCATION, "/elsewhere.json")]).into_response()
        }
        Mode::Hang => {
            tokio::time::sleep(Duration::from_secs(60)).await;
            StatusCode::OK.into_response()
        }
        Mode::Oversized => {
            let pad = "x".repeat(512 * 1024);
            axum::Json(json!({ "keys": [], "pad": pad })).into_response()
        }
        Mode::Garbage => (StatusCode::OK, "<html>not json</html>").into_response(),
    }
}

impl TestIdp {
    pub async fn start() -> Self {
        let key = TestKey::generate();
        let state = Arc::new(Mutex::new(IdpState {
            keys: vec![key.public_jwk()],
            mode: Mode::Normal,
        }));
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/jwks.json", get(jwks))
            .with_state(ServerState {
                state: Arc::clone(&state),
                hits: Arc::clone(&hits),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind idp");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            state,
            hits,
            addr,
            key: Mutex::new(key),
        }
    }

    pub fn jwks_url(&self) -> String {
        format!("http://{}/jwks.json", self.addr)
    }

    pub fn jwks_hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn set_mode(&self, mode: Mode) {
        self.state.lock().expect("idp state").mode = mode;
    }

    /// Publish exactly these keys.
    pub fn publish(&self, keys: Vec<Value>) {
        self.state.lock().expect("idp state").keys = keys;
    }

    /// Introduce a new signing key. With `keep_old` the previous key stays published (graceful
    /// rotation); without it the old key disappears (emergency revocation of a key).
    pub fn rotate(&self, keep_old: bool) -> TestKey {
        let new_key = TestKey::generate();
        let mut keys = vec![new_key.public_jwk()];
        if keep_old {
            keys.push(self.key.lock().expect("key").public_jwk());
        }
        self.publish(keys);
        *self.key.lock().expect("key") = new_key.clone();
        new_key
    }

    /// A token builder pre-filled with valid claims for the current key.
    pub fn token(&self, subject: &str, tenant: &str) -> TokenBuilder {
        TokenBuilder::new(self.key.lock().expect("key").clone(), subject, tenant)
    }
}

/// Builds and signs a JWT, letting tests break exactly one thing at a time.
pub struct TokenBuilder {
    key: TestKey,
    header_alg: Algorithm,
    kid: Option<String>,
    claims: Map<String, Value>,
    secret_override: Option<Vec<u8>>,
}

impl TokenBuilder {
    pub fn new(key: TestKey, subject: &str, tenant: &str) -> Self {
        let now = now_secs();
        let mut claims = Map::new();
        claims.insert("iss".into(), json!(ISSUER));
        claims.insert("aud".into(), json!(AUDIENCE));
        claims.insert("sub".into(), json!(subject));
        claims.insert("tenant".into(), json!(tenant));
        claims.insert("iat".into(), json!(now));
        claims.insert("exp".into(), json!(now + 300));
        let kid = Some(key.kid.clone());
        Self {
            key,
            header_alg: Algorithm::ES256,
            kid,
            claims,
            secret_override: None,
        }
    }

    pub fn claim(mut self, name: &str, value: Value) -> Self {
        self.claims.insert(name.into(), value);
        self
    }

    pub fn without(mut self, name: &str) -> Self {
        self.claims.remove(name);
        self
    }

    pub fn kid(mut self, kid: Option<&str>) -> Self {
        self.kid = kid.map(str::to_owned);
        self
    }

    pub fn signed_with(mut self, key: TestKey) -> Self {
        self.key = key;
        self
    }

    /// Issue `iat` this many seconds in the past and expire `lifetime` seconds after it.
    pub fn issued(self, seconds_ago: u64, lifetime: u64) -> Self {
        let iat = now_secs().saturating_sub(seconds_ago);
        self.claim("iat", json!(iat))
            .claim("exp", json!(iat + lifetime))
    }

    /// HS256 token whose HMAC secret is the given bytes (key-confusion attempt).
    pub fn hmac_with(mut self, secret: Vec<u8>) -> Self {
        self.header_alg = Algorithm::HS256;
        self.secret_override = Some(secret);
        self
    }

    pub fn build(self) -> String {
        let mut header = Header::new(self.header_alg);
        header.kid = self.kid;
        let key = match &self.secret_override {
            Some(secret) => EncodingKey::from_secret(secret),
            None => self.key.encoding_key(),
        };
        jsonwebtoken::encode(&header, &Value::Object(self.claims), &key).expect("sign token")
    }
}

/// `header.payload.` with `alg: none` and an empty signature.
pub fn unsigned_token(kid: &str, claims: &Value) -> String {
    let header = json!({ "alg": "none", "typ": "JWT", "kid": kid });
    format!(
        "{}.{}.",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

/// Replaces the payload of a signed token without re-signing it.
pub fn tamper_payload(token: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut parts: Vec<&str> = token.split('.').collect();
    let decoded = URL_SAFE_NO_PAD.decode(parts[1]).expect("payload b64");
    let mut payload: Value = serde_json::from_slice(&decoded).expect("payload json");
    edit(&mut payload);
    let replaced = URL_SAFE_NO_PAD.encode(payload.to_string());
    parts[1] = &replaced;
    parts.join(".")
}

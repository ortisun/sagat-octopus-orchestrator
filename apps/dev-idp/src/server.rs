//! The three things a development stack asks of an identity provider: the keys that verify its
//! tokens, where to find them, and a token for a subject of a tenant.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use sagat_domain::identity::{Slug, Subject};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{Config, MAX_TTL_SECS};
use crate::key::DevKey;

/// A request to this provider is a few dozen bytes.
const MAX_BODY: usize = 4096;

struct Inner {
    config: Config,
    hosts: Vec<String>,
    key: DevKey,
}

type Shared = Arc<Inner>;

/// The provider as a router.
pub fn router(config: Config, key: DevKey) -> Router {
    let state: Shared = Arc::new(Inner {
        hosts: config.hosts(),
        config,
        key,
    });
    Router::new()
        .route("/livez", get(livez))
        .route("/.well-known/jwks.json", get(jwks))
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/token", post(token))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(state)
}

fn reply(status: StatusCode, body: &Value) -> Response {
    let mut response = (status, body.to_string()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn refuse(status: StatusCode, error: &str, message: &str) -> Response {
    reply(status, &json!({ "error": error, "message": message }))
}

/// A request whose `Host` is not one this provider is known by is refused: a page that makes a
/// browser resolve another name to this address must not be able to ask for a token.
fn host_known(state: &Inner, headers: &HeaderMap) -> bool {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    state.hosts.iter().any(|known| known == host)
}

fn unknown_host() -> Response {
    refuse(
        StatusCode::FORBIDDEN,
        "host_not_allowed",
        "this provider answers only to the addresses it is configured with",
    )
}

async fn livez() -> &'static str {
    "ok"
}

async fn jwks(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if !host_known(&state, &headers) {
        return unknown_host();
    }
    reply(StatusCode::OK, &json!({ "keys": [state.key.public_jwk()] }))
}

async fn discovery(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if !host_known(&state, &headers) {
        return unknown_host();
    }
    let base = format!("{}/", state.config.issuer.trim_end_matches('/'));
    reply(
        StatusCode::OK,
        &json!({
            "issuer": state.config.issuer,
            "jwks_uri": format!("{base}.well-known/jwks.json"),
            "token_endpoint": format!("{base}token"),
            "id_token_signing_alg_values_supported": ["ES256"],
            "subject_types_supported": ["public"],
            "response_types_supported": ["token"],
        }),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TokenRequest {
    subject: String,
    tenant: String,
    #[serde(default)]
    ttl_seconds: Option<u64>,
}

async fn token(State(state): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    if !host_known(&state, &headers) {
        return unknown_host();
    }
    // Only JSON: a form or a text body is what a page of another origin can send without asking.
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return refuse(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "send the request as application/json",
        );
    }
    let Ok(request) = serde_json::from_slice::<TokenRequest>(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "expected {\"subject\": ..., \"tenant\": ..., \"ttlSeconds\": ...} (ttlSeconds is optional)",
        );
    };
    let Ok(subject) = Subject::parse(&request.subject) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_subject",
            "not a subject the platform accepts",
        );
    };
    let Ok(tenant) = Slug::parse(&request.tenant) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_tenant",
            "not a tenant slug",
        );
    };
    let ttl = request.ttl_seconds.unwrap_or(MAX_TTL_SECS);
    if ttl == 0 || ttl > MAX_TTL_SECS {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_ttl",
            &format!("ttlSeconds is between 1 and {MAX_TTL_SECS}"),
        );
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut claims = serde_json::Map::new();
    claims.insert("iss".into(), json!(state.config.issuer));
    claims.insert("aud".into(), json!(state.config.audience));
    claims.insert("sub".into(), json!(subject.as_str()));
    claims.insert(state.config.tenant_claim.clone(), json!(tenant.as_str()));
    claims.insert("iat".into(), json!(now));
    claims.insert("exp".into(), json!(now + ttl));
    match state.key.sign(&Value::Object(claims)) {
        Ok(access_token) => {
            // Who it was for and for how long; never the token.
            tracing::info!(subject = %subject.as_str(), tenant = %tenant.as_str(), ttl, "issued a token");
            reply(
                StatusCode::OK,
                &json!({ "access_token": access_token, "token_type": "Bearer", "expires_in": ttl }),
            )
        }
        Err(_) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "signing_failed",
            "the token could not be signed",
        ),
    }
}

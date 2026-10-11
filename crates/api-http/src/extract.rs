//! Request extractors. [`Caller`] is the only way a handler obtains an authenticated session, and
//! [`ApiJson`] maps body-parsing failures into the API's error envelope.

use std::error::Error as _;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use sagat_application::identity::{RequestContext, Session};
use sagat_telemetry::TraceContext;
use serde::de::DeserializeOwned;
use tower_http::timeout::TimeoutError;

use crate::concurrency::TenantPermit;
use crate::core::Core;
use crate::error::ApiError;

/// An authenticated caller plus the correlation data of its request.
///
/// It also owns the caller's slot in its tenant's concurrency cap, so the slot is held exactly as
/// long as the handler that received the `Caller` runs (including while it reads a slow body).
#[derive(Debug)]
pub struct Caller {
    pub session: Session,
    pub ctx: RequestContext,
    pub trace: TraceContext,
    /// When the bearer token expires, in seconds since the Unix epoch.
    pub token_expires_at: u64,
    _slot: TenantPermit,
}

/// What authenticating a bearer token yields: the session and the caller's slot in its tenant's
/// concurrency cap (held for as long as the value lives).
#[derive(Debug)]
pub struct Authenticated {
    pub session: Session,
    pub slot: TenantPermit,
    /// When the bearer token expires, in seconds since the Unix epoch.
    pub expires_at: u64,
}

/// The one sequence that decides who a bearer token belongs to, shared by every transport:
/// verify the token, charge the per-subject budget BEFORE touching the database, look the caller up,
/// charge the per-tenant budget, take a slot of the tenant's concurrency cap.
///
/// # Errors
/// [`ApiError`] for a bad token (401), an exhausted budget (429), an unknown member (401) or an
/// unreachable dependency (503).
pub async fn authenticate_bearer<St: Core>(
    state: &St,
    token: &str,
) -> Result<Authenticated, ApiError> {
    let claims = state.verifier().verify(token).await?;
    // Budget per verified subject, before the database is touched: `authenticate` costs two
    // lookups, and a holder of a valid token who is not a member (or names an unknown tenant)
    // would otherwise get them for free, without ever reaching the tenant budget below. Only a
    // token that verified is charged, so an anonymous flood cannot spend anyone's budget.
    if let Err(retry_after) = state.subject_limiter().check(claims.subject.clone()) {
        metrics::counter!("http_rate_limited_total", "scope" => "subject").increment(1);
        return Err(ApiError::rate_limited(retry_after));
    }
    let session = state.authenticate(&claims.tenant, &claims.subject).await?;
    // Budget per tenant, charged only to callers that proved who they are: an anonymous flood
    // cannot spend (or be blamed on) a tenant's budget.
    if let Err(retry_after) = state.tenant_limiter().check(session.principal.tenant_id()) {
        metrics::counter!("http_rate_limited_total", "scope" => "tenant").increment(1);
        return Err(ApiError::rate_limited(retry_after));
    }
    // How many requests of this tenant may be in flight at once: a tenant that parks requests
    // (a stalled upload) can fill only its own share of the global slots.
    let slot = state
        .tenant_concurrency()
        .try_acquire(session.principal.tenant_id())
        .map_err(|_| {
            metrics::counter!("http_tenant_concurrency_rejected_total").increment(1);
            ApiError::rate_limited(Duration::from_secs(1))
        })?;
    Ok(Authenticated {
        session,
        slot,
        expires_at: claims.expires_at,
    })
}

impl<St: Core> FromRequestParts<St> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &St) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let Authenticated {
            session,
            slot,
            expires_at,
        } = authenticate_bearer(state, token).await?;
        let trace = parts
            .extensions
            .get::<TraceContext>()
            .copied()
            .unwrap_or_else(|| TraceContext::from_traceparent(None));
        let ctx = RequestContext {
            trace_id: Some(trace.trace_id().to_string()),
            span_id: parts
                .extensions
                .get::<crate::context::IngressSpan>()
                .map(|ingress| ingress.0.to_string()),
        };
        Ok(Self {
            session,
            ctx,
            trace,
            token_expires_at: expires_at,
            _slot: slot,
        })
    }
}

/// Extracts the token of a single `Authorization: Bearer <token>` header.
fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        // Missing, or ambiguous because several headers were sent.
        return Err(ApiError::missing_credentials());
    };
    parse_bearer(
        value
            .to_str()
            .map_err(|_| ApiError::invalid_credentials())?,
    )
}

/// The token of an `Authorization` value of the form `Bearer <token>` (any transport).
///
/// # Errors
/// [`ApiError`] (401) for any other shape.
pub fn parse_bearer(value: &str) -> Result<&str, ApiError> {
    let (scheme, token) = value
        .split_once(' ')
        .ok_or_else(ApiError::invalid_credentials)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.bytes().any(|b| b.is_ascii_whitespace())
    {
        return Err(ApiError::invalid_credentials());
    }
    Ok(token)
}

/// Whether reading the body failed because the idle timeout fired (somewhere in the error chain).
fn is_body_timeout(rejection: &JsonRejection) -> bool {
    let mut source = rejection.source();
    while let Some(error) = source {
        if error.is::<TimeoutError>() {
            return true;
        }
        source = error.source();
    }
    false
}

/// `axum::Json` with failures reported in the API's error envelope.
pub struct ApiJson<T>(pub T);

impl<T, St> FromRequest<St> for ApiJson<T>
where
    T: DeserializeOwned,
    St: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &St) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            // The client stopped sending the body (see `HttpLimits::body_idle_timeout`).
            Err(rejection) if is_body_timeout(&rejection) => Err(ApiError::request_timeout()),
            Err(rejection) => Err(match rejection {
                JsonRejection::JsonDataError(_) => {
                    ApiError::validation("request body does not match the expected schema")
                }
                JsonRejection::JsonSyntaxError(_) => {
                    ApiError::bad_request("request body is not valid JSON")
                }
                JsonRejection::MissingJsonContentType(_) => {
                    ApiError::bad_request("content type must be application/json")
                        .with_status(StatusCode::UNSUPPORTED_MEDIA_TYPE)
                }
                other => ApiError::bad_request("request body could not be read")
                    .with_status(other.status()),
            }),
        }
    }
}

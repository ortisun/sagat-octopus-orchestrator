//! Request extractors. [`Caller`] is the only way a handler obtains an authenticated session, and
//! [`ApiJson`] maps body-parsing failures into the API's error envelope.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use sagat_application::clock::Clock;
use sagat_application::identity::{IdentityStore, RequestContext, Session};
use sagat_telemetry::TraceContext;
use serde::de::DeserializeOwned;

use crate::AppState;
use crate::error::ApiError;

/// An authenticated caller plus the correlation data of its request.
#[derive(Debug)]
pub struct Caller {
    pub session: Session,
    pub ctx: RequestContext,
    pub trace: TraceContext,
}

impl<S, C> FromRequestParts<AppState<S, C>> for Caller
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<S, C>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let claims = state.verifier.verify(token).await?;
        let session = state
            .identity
            .authenticate(&claims.tenant, &claims.subject)
            .await?;
        let trace = parts
            .extensions
            .get::<TraceContext>()
            .copied()
            .unwrap_or_else(|| TraceContext::from_traceparent(None));
        let ctx = RequestContext {
            trace_id: Some(trace.trace_id().to_string()),
        };
        Ok(Self {
            session,
            ctx,
            trace,
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
    let value = value
        .to_str()
        .map_err(|_| ApiError::invalid_credentials())?;
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

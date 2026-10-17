//! HTTP resource for namespace-scoped PII reveal grants.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::routing::{get, put};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::reveal_grants::{
    DEFAULT_PAGE_SIZE, RevealGrant, RevealGrantError, RevealGrantService, RevealGrantStore,
};
use sagat_domain::identity::{NamespaceId, Subject};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::admission::{Admission, admit_resolved_namespace};
use crate::core::Core;
use crate::error::ApiError;
use crate::extract::Caller;

/// State contract needed by the reveal-grant handlers.
///
/// The control-plane composition implements this on its shared state and mounts [`routes`]. Keeping
/// the contract here lets the resource remain independently testable without owning router
/// composition.
pub trait RevealGrantApi: Core + Admission {
    type GrantStore: RevealGrantStore + 'static;
    type GrantClock: Clock + 'static;

    fn reveal_grants(&self) -> &RevealGrantService<Self::GrantStore, Self::GrantClock>;
}

/// Builds the three grant-resource routes. The caller composes this router under `/v1` and applies
/// the shared protective layers.
pub fn routes<B, T>(state: B) -> Router<T>
where
    B: RevealGrantApi,
    T: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/namespaces/{namespace_id}/payload-reveal-grants",
            get(list_payload_reveal_grants::<B>),
        )
        .route(
            "/namespaces/{namespace_id}/payload-reveal-grants/{subject}",
            put(grant_payload_reveal::<B>).delete(revoke_payload_reveal::<B>),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevealGrantQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevealGrantBody {
    pub subject: String,
    pub granted_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub granted_at: OffsetDateTime,
}

impl From<&RevealGrant> for RevealGrantBody {
    fn from(grant: &RevealGrant) -> Self {
        Self {
            subject: grant.subject.as_str().to_owned(),
            granted_by: grant.granted_by.as_str().to_owned(),
            granted_at: grant.granted_at,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevealGrantListBody {
    pub items: Vec<RevealGrantBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EmptyBody {}

fn no_store_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn decode_cursor(cursor: &str) -> Result<Subject, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    let subject =
        String::from_utf8(bytes).map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    Subject::parse(&subject).map_err(|_| ApiError::bad_request("the cursor is not valid"))
}

fn encode_cursor(subject: &Subject) -> String {
    URL_SAFE_NO_PAD.encode(subject.as_str().as_bytes())
}

fn map_reveal_error(error: RevealGrantError) -> ApiError {
    match error {
        RevealGrantError::Application(error) => ApiError::from(error),
        RevealGrantError::UnknownSubject => {
            ApiError::validation("the subject is not a member of this tenant")
                .with_reason("UNKNOWN_SUBJECT")
        }
        RevealGrantError::RoleCannotReveal => {
            ApiError::validation("the subject's current role cannot read internal payload values")
                .with_reason("ROLE_CANNOT_REVEAL")
        }
        RevealGrantError::GrantLimit => {
            ApiError::from(AppError::QuotaExceeded("payload reveal grants"))
                .with_reason("GRANT_LIMIT")
        }
    }
}

pub async fn list_payload_reveal_grants<B: RevealGrantApi>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<RevealGrantQuery>,
) -> Result<(HeaderMap, Json<RevealGrantListBody>), ApiError> {
    let namespace = namespace_id(&namespace)?;
    let grants = state.reveal_grants();
    let scope = grants
        .prepare(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    admit_resolved_namespace(&state, &caller, scope.namespace_id()).await?;
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let page = grants
        .list(
            &scope,
            after.as_ref(),
            query.limit.unwrap_or(DEFAULT_PAGE_SIZE),
        )
        .await
        .map_err(map_reveal_error)?;
    let next_cursor = page.next_after.as_ref().map(encode_cursor);
    Ok((
        no_store_headers(),
        Json(RevealGrantListBody {
            items: page.items.iter().map(RevealGrantBody::from).collect(),
            next_cursor,
        }),
    ))
}

pub async fn grant_payload_reveal<B: RevealGrantApi>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
) -> Result<(HeaderMap, Json<RevealGrantBody>), ApiError> {
    let namespace = namespace_id(&namespace)?;
    let grants = state.reveal_grants();
    let scope = grants
        .prepare(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    admit_resolved_namespace(&state, &caller, scope.namespace_id()).await?;
    let grant = grants
        .grant(&scope, &subject, &caller.ctx)
        .await
        .map_err(map_reveal_error)?;
    Ok((no_store_headers(), Json(RevealGrantBody::from(&grant))))
}

pub async fn revoke_payload_reveal<B: RevealGrantApi>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
) -> Result<(HeaderMap, Json<EmptyBody>), ApiError> {
    let namespace = namespace_id(&namespace)?;
    let grants = state.reveal_grants();
    let scope = grants
        .prepare(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    admit_resolved_namespace(&state, &caller, scope.namespace_id()).await?;
    if !grants
        .revoke(&scope, &subject, &caller.ctx)
        .await
        .map_err(map_reveal_error)?
    {
        return Err(ApiError::not_found());
    }
    Ok((no_store_headers(), Json(EmptyBody {})))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use axum::http::StatusCode;
    use sagat_application::reveal_grants::{RevealGrant, RevealGrantError};
    use time::OffsetDateTime;

    use super::{
        EmptyBody, RevealGrantBody, RevealGrantListBody, decode_cursor, encode_cursor,
        map_reveal_error,
    };
    use sagat_domain::identity::Subject;

    #[test]
    fn cursor_round_trips_non_uuid_subjects_without_exposing_the_value() {
        let subject = Subject::parse("oidc|developer@example.test").expect("subject");
        let encoded = encode_cursor(&subject);
        assert!(!encoded.contains(subject.as_str()));
        assert_eq!(decode_cursor(&encoded).expect("cursor"), subject);
        assert!(decode_cursor("not a cursor!").is_err());
    }

    #[test]
    fn closed_grant_errors_use_the_contract_status_and_reason() {
        for (error, status, reason) in [
            (
                RevealGrantError::UnknownSubject,
                StatusCode::UNPROCESSABLE_ENTITY,
                "UNKNOWN_SUBJECT",
            ),
            (
                RevealGrantError::RoleCannotReveal,
                StatusCode::UNPROCESSABLE_ENTITY,
                "ROLE_CANNOT_REVEAL",
            ),
            (
                RevealGrantError::GrantLimit,
                StatusCode::TOO_MANY_REQUESTS,
                "GRANT_LIMIT",
            ),
        ] {
            let mapped = map_reveal_error(error);
            assert_eq!(mapped.parts().status, status);
            assert_eq!(
                mapped
                    .parts()
                    .details
                    .as_ref()
                    .and_then(|details| details.get("reason"))
                    .and_then(serde_json::Value::as_str),
                Some(reason)
            );
        }
    }

    #[test]
    fn response_uses_camel_case_rfc3339_and_an_empty_revoke_object() {
        let grant = RevealGrant {
            subject: Subject::parse("developer-1").expect("subject"),
            granted_by: Subject::parse("admin-1").expect("actor"),
            granted_at: OffsetDateTime::UNIX_EPOCH,
        };
        assert_eq!(
            serde_json::to_value(RevealGrantBody::from(&grant)).expect("grant body"),
            serde_json::json!({
                "subject": "developer-1",
                "grantedBy": "admin-1",
                "grantedAt": "1970-01-01T00:00:00Z"
            })
        );
        assert_eq!(
            serde_json::to_value(RevealGrantListBody {
                items: Vec::new(),
                next_cursor: None,
            })
            .expect("list body"),
            serde_json::json!({"items": []})
        );
        assert_eq!(
            serde_json::to_value(EmptyBody {}).expect("revoke body"),
            serde_json::json!({})
        );
    }
}

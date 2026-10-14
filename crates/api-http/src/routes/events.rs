//! Events: the endpoints that authenticate webhooks, publishing through the API, and reading the
//! inbox.
//!
//! Webhook senders use the endpoint's HMAC, external OIDC or verified native mTLS profile instead
//! of platform membership. Invalid identities share a uniform `401`; verifier outages are retryable.

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::error::AppError;
use sagat_application::events::{EndpointSettings, PublishRequest, WebhookRequest};
use sagat_application::identity::TraceLink;
use sagat_domain::events::{
    DEFAULT_DEDUPE_TTL, DEFAULT_MAX_BODY, DEFAULT_RATE, DEFAULT_SKEW, EndpointId, EndpointLimits,
    EndpointStatus, EventId, EventKey,
};
use sagat_domain::execution::CorrelationKey;
use sagat_domain::identity::{NamespaceId, Slug};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use sagat_telemetry::TraceContext;

use crate::admission::{admit, namespace_slug};
use crate::dto_events::{
    AcceptedBody, CreateEndpointBody, EndpointBody, EndpointListBody, EventBody, PublishEventBody,
    UpdateEndpointBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

/// Largest body of the event routes: the data (240 KiB) and a little room for the envelope.
pub const EVENT_MAX_BODY_BYTES: usize = 240 * 1024 + 4096;

const NAMESPACE_HEADER: &str = "x-octopus-namespace";
const SIGNATURE_HEADER: &str = "x-octopus-signature";
const TIMESTAMP_HEADER: &str = "x-octopus-timestamp";
const EVENT_ID_HEADER: &str = "x-octopus-event-id";
const EVENT_TYPE_HEADER: &str = "x-octopus-event-type";
const SUBJECT_HEADER: &str = "x-octopus-subject";
const CORRELATION_HEADER: &str = "x-octopus-correlation-key";

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn endpoint_id(raw: &str) -> Result<EndpointId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn events<B: Governance>(
    state: &B,
) -> Result<&sagat_application::events::EventService<B::Store, B::Clock>, ApiError> {
    state.events().ok_or_else(ApiError::not_found)
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::validation(message)
}

async fn admitted_namespace<B: Governance>(
    state: &B,
    caller: &Caller,
    raw: &str,
    operation: ApiOperation,
) -> Result<NamespaceId, ApiError> {
    let namespace = namespace_id(raw)?;
    admit(state, caller, operation, ApiTarget::Namespace(namespace)).await?;
    Ok(namespace)
}

async fn admitted_endpoint<B: Governance>(
    state: &B,
    caller: &Caller,
    raw_namespace: &str,
    raw_endpoint: &str,
    operation: ApiOperation,
) -> Result<sagat_application::events::EndpointRecord, ApiError> {
    let namespace = namespace_id(raw_namespace)?;
    let endpoint = events(state)?
        .get_endpoint(&caller.session.principal, endpoint_id(raw_endpoint)?)
        .await?;
    if endpoint.namespace_id != namespace {
        return Err(ApiError::not_found());
    }
    admit(
        state,
        caller,
        operation,
        ApiTarget::Namespace(endpoint.namespace_id),
    )
    .await?;
    Ok(endpoint)
}

/// The settings of a request, with the defaults of an endpoint for what it leaves out.
fn settings(
    credential: (
        Option<&str>,
        Option<crate::dto_events::WebhookAuthenticationBody>,
    ),
    default_event_type: Option<&str>,
    dedupe_ttl_seconds: Option<u64>,
    skew_seconds: Option<u64>,
    max_body_bytes: Option<usize>,
    rate_per_second: Option<u32>,
    status: Option<&str>,
) -> Result<EndpointSettings, ApiError> {
    let (secret_name, authentication) = credential;
    let limits = EndpointLimits::new(
        dedupe_ttl_seconds.map_or(DEFAULT_DEDUPE_TTL, Duration::from_secs),
        skew_seconds.map_or(DEFAULT_SKEW, Duration::from_secs),
        max_body_bytes.unwrap_or(DEFAULT_MAX_BODY),
        rate_per_second.unwrap_or(DEFAULT_RATE),
    )
    .map_err(|e| ApiError::validation(e.to_string()))?;
    Ok(EndpointSettings {
        secret_name: secret_name
            .map(SecretName::parse)
            .transpose()
            .map_err(|_| invalid("the secret name is not valid"))?,
        authentication: match authentication {
            None | Some(crate::dto_events::WebhookAuthenticationBody::Hmac) => {
                sagat_domain::events::WebhookAuthentication::Hmac
            }
            Some(crate::dto_events::WebhookAuthenticationBody::Oidc { profile, subject }) => {
                sagat_domain::events::WebhookAuthentication::Oidc {
                    profile: Slug::parse(&profile)
                        .map_err(|_| invalid("the OIDC profile is not valid"))?,
                    subject,
                }
            }
            Some(crate::dto_events::WebhookAuthenticationBody::Mtls { certificate_sha256 }) => {
                sagat_domain::events::WebhookAuthentication::Mtls { certificate_sha256 }
            }
        },
        default_event_type: default_event_type
            .map(|t| {
                EventType::parse(t).map_err(|_| invalid("the default event type is not valid"))
            })
            .transpose()?,
        limits,
        status: match status {
            None => EndpointStatus::Active,
            Some(text) => text
                .parse()
                .map_err(|()| invalid("the status is ACTIVE or PAUSED"))?,
        },
    })
}

// ------------------------------------------------------------------------------ endpoints

pub async fn create_endpoint<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<CreateEndpointBody>,
) -> Result<(StatusCode, Json<EndpointBody>), ApiError> {
    let settings = settings(
        (body.secret_name.as_deref(), body.authentication),
        body.default_event_type.as_deref(),
        body.dedupe_ttl_seconds,
        body.skew_seconds,
        body.max_body_bytes,
        body.rate_per_second,
        body.status.as_deref(),
    )?;
    let slug = Slug::parse(&body.slug).map_err(|_| invalid("the slug is not valid"))?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventManage).await?;
    let endpoint = events(&state)?
        .create_endpoint(
            &caller.session.principal,
            namespace,
            slug,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(EndpointBody::from(&endpoint))))
}

pub async fn list_endpoints<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<EndpointListBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventRead).await?;
    let items = events(&state)?
        .list_endpoints(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(EndpointListBody {
        items: items.iter().map(EndpointBody::from).collect(),
    }))
}

pub async fn get_endpoint<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<EndpointBody>, ApiError> {
    let endpoint =
        admitted_endpoint(&state, &caller, &namespace, &id, ApiOperation::EventRead).await?;
    Ok(Json(EndpointBody::from(&endpoint)))
}

pub async fn update_endpoint<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateEndpointBody>,
) -> Result<Json<EndpointBody>, ApiError> {
    let settings = settings(
        (body.secret_name.as_deref(), body.authentication),
        body.default_event_type.as_deref(),
        body.dedupe_ttl_seconds,
        body.skew_seconds,
        body.max_body_bytes,
        body.rate_per_second,
        body.status.as_deref(),
    )?;
    admitted_endpoint(&state, &caller, &namespace, &id, ApiOperation::EventManage).await?;
    let endpoint = events(&state)?
        .update_endpoint(
            &caller.session.principal,
            endpoint_id(&id)?,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok(Json(EndpointBody::from(&endpoint)))
}

pub async fn delete_endpoint<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    admitted_endpoint(&state, &caller, &namespace, &id, ApiOperation::EventManage).await?;
    events(&state)?
        .delete_endpoint(&caller.session.principal, endpoint_id(&id)?, &caller.ctx)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------------------ the inbox

pub async fn publish_event<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<PublishEventBody>,
) -> Result<(StatusCode, Json<AcceptedBody>), ApiError> {
    let namespace = headers
        .get(NAMESPACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 63)
        .ok_or_else(|| ApiError::bad_request("the X-Octopus-Namespace header is required"))?;
    let data = serde_json::to_vec(&body.data)
        .map_err(|_| ApiError::bad_request("the event data cannot be encoded"))?;
    let request = PublishRequest {
        namespace: namespace.to_owned(),
        event_id: body
            .event_id
            .as_deref()
            .map(EventId::parse)
            .transpose()
            .map_err(|e| ApiError::validation(e.to_string()))?,
        event_type: EventType::parse(&body.event_type)
            .map_err(|_| invalid("the event type is not valid"))?,
        subject: body.subject,
        correlation_key: body
            .correlation_key
            .as_deref()
            .map(CorrelationKey::parse)
            .transpose()
            .map_err(|_| invalid("the correlation key is not usable"))?,
        occurred_at: body.occurred_at,
        data,
        dedupe_ttl: body.dedupe_ttl_seconds.map(Duration::from_secs),
    };
    admit(
        &state,
        &caller,
        ApiOperation::EventPublish,
        ApiTarget::NamespaceSlug(namespace_slug(namespace)?),
    )
    .await?;
    let received = events(&state)?
        .publish(&caller.session.principal, request, &caller.ctx)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(AcceptedBody::from(&received))))
}

pub async fn get_event<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, source, event_id)): Path<(String, String, String)>,
) -> Result<Json<EventBody>, ApiError> {
    let key = EventKey {
        source: Slug::parse(&source).map_err(|_| ApiError::not_found())?,
        event_id: EventId::parse(&event_id).map_err(|_| ApiError::not_found())?,
    };
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventRead).await?;
    let event = events(&state)?
        .event(&caller.session.principal, namespace, &key, &caller.ctx)
        .await?;
    Ok(Json(EventBody::from(&event)))
}

// ------------------------------------------------------------------------------ the public URL

/// The value of a header that was sent exactly once and is text; anything else is as good as absent
/// (a request with two signatures is not one a provider sends).
fn single<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (Some(value), None) => value.to_str().ok(),
        _ => None,
    }
}

/// `POST /webhooks/{endpointId}`. There is no `Caller` here by design.
pub async fn receive_webhook<B: Governance>(
    State(state): State<B>,
    Path(endpoint): Path<String>,
    trace: Option<axum::Extension<TraceContext>>,
    certificate: Option<axum::Extension<crate::server::VerifiedClientCertificate>>,
    ingress: Option<axum::Extension<crate::context::IngressSpan>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<AcceptedBody>), ApiError> {
    // An id that is not an id gets the answer an unknown endpoint gets.
    let Ok(id) = endpoint.parse::<EndpointId>() else {
        metrics::counter!("webhook_rejected_total", "reason" => "unknown_endpoint").increment(1);
        return Err(ApiError::webhook_rejected());
    };
    let Some(service) = state.events() else {
        return Err(ApiError::not_found());
    };
    // The caller's trace, when it sent a `traceparent`: the run this event starts continues it.
    let link = trace.filter(|t| t.is_continuation()).map(|t| TraceLink {
        trace_id: t.trace_id().to_string(),
        parent_span: ingress.map(|i| i.0.0.to_string()),
    });
    let request = WebhookRequest {
        signature: single(&headers, SIGNATURE_HEADER),
        bearer: single(&headers, "authorization").and_then(|s| s.strip_prefix("Bearer ")),
        certificate_sha256: certificate.as_ref().map(|cert| cert.sha256.as_str()),
        timestamp: single(&headers, TIMESTAMP_HEADER),
        event_id: single(&headers, EVENT_ID_HEADER),
        event_type: single(&headers, EVENT_TYPE_HEADER),
        subject: single(&headers, SUBJECT_HEADER),
        correlation_key: single(&headers, CORRELATION_HEADER),
        body: &body,
        trace: link.as_ref(),
    };
    match service.receive(id, &request).await {
        Ok(received) => Ok((StatusCode::ACCEPTED, Json(AcceptedBody::from(&received)))),
        Err(AppError::Unauthenticated) => Err(ApiError::webhook_rejected()),
        Err(other) => Err(other.into()),
    }
}

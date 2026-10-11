//! Connectors to the brokers of a tenant, and the messages they set aside.
//!
//! A connector is addressed through its namespace, and one that belongs to another tenant is a `404`
//! like one that does not exist.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sagat_application::connectors::ConnectorChoices;
use sagat_domain::connectors::{
    ConnectorId, ConnectorKind, ConnectorSettings, ConnectorStatus, StartFrom,
};
use sagat_domain::identity::{NamespaceId, Slug};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::dto_events::{
    AcceptedBody, ConnectorBody, ConnectorListBody, CreateConnectorBody, DeadLetterBody,
    DeadLetterPageBody, KafkaSettingsBody, NatsSettingsBody, SqsSettingsBody, UpdateConnectorBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn connector_id(raw: &str) -> Result<ConnectorId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn letter_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn connectors<B: Governance>(
    state: &B,
) -> Result<&sagat_application::connectors::ConnectorService<B::Store, B::Clock>, ApiError> {
    state.connectors().ok_or_else(ApiError::not_found)
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::validation(message)
}

/// The settings of the kind named, from the JSON a caller sent.
fn settings(kind: &str, value: &Value) -> Result<ConnectorSettings, ApiError> {
    let kind = kind
        .parse::<ConnectorKind>()
        .map_err(|()| invalid("the kind is NATS, KAFKA or SQS"))?;
    let bad = |_| invalid("the settings do not fit the kind of connector");
    let refused = |e: sagat_domain::connectors::ConnectorError| ApiError::validation(e.to_string());
    match kind {
        ConnectorKind::Nats => {
            let s: NatsSettingsBody = serde_json::from_value(value.clone()).map_err(bad)?;
            ConnectorSettings::nats(
                &s.servers,
                &s.stream,
                &s.consumer,
                s.filter_subject.as_deref(),
            )
            .map_err(refused)
        }
        ConnectorKind::Kafka => {
            let s: KafkaSettingsBody = serde_json::from_value(value.clone()).map_err(bad)?;
            let start = match s.start_from.as_deref() {
                None | Some("earliest") => StartFrom::Earliest,
                Some("latest") => StartFrom::Latest,
                Some(_) => return Err(invalid("startFrom is earliest or latest")),
            };
            ConnectorSettings::kafka(&s.servers, &s.topic, start).map_err(refused)
        }
        ConnectorKind::Sqs => {
            let s: SqsSettingsBody = serde_json::from_value(value.clone()).map_err(bad)?;
            ConnectorSettings::sqs(&s.queue_url, &s.region).map_err(refused)
        }
    }
}

fn choices(
    kind: &str,
    settings_value: &Value,
    secret_name: Option<&str>,
    default_event_type: Option<&str>,
    rate_per_second: Option<u32>,
    status: Option<&str>,
) -> Result<ConnectorChoices, ApiError> {
    Ok(ConnectorChoices {
        settings: settings(kind, settings_value)?,
        secret_name: secret_name
            .map(|s| SecretName::parse(s).map_err(|_| invalid("the secret name is not valid")))
            .transpose()?,
        default_event_type: default_event_type
            .map(|t| {
                EventType::parse(t).map_err(|_| invalid("the default event type is not valid"))
            })
            .transpose()?,
        rate_per_second,
        status: match status {
            None => ConnectorStatus::Active,
            Some(text) => text
                .parse()
                .map_err(|()| invalid("the status is ACTIVE or PAUSED"))?,
        },
    })
}

pub async fn create_connector<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<CreateConnectorBody>,
) -> Result<(StatusCode, Json<ConnectorBody>), ApiError> {
    let choices = choices(
        &body.kind,
        &body.settings,
        body.secret_name.as_deref(),
        body.default_event_type.as_deref(),
        body.rate_per_second,
        body.status.as_deref(),
    )?;
    let slug = Slug::parse(&body.slug).map_err(|_| invalid("the slug is not valid"))?;
    let created = connectors(&state)?
        .create(
            &caller.session.principal,
            namespace_id(&namespace)?,
            slug,
            choices,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(ConnectorBody::from(&created))))
}

pub async fn list_connectors<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<ConnectorListBody>, ApiError> {
    let items = connectors(&state)?
        .list(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ConnectorListBody {
        items: items.iter().map(ConnectorBody::from).collect(),
    }))
}

pub async fn get_connector<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id)): Path<(String, String)>,
) -> Result<Json<ConnectorBody>, ApiError> {
    let found = connectors(&state)?
        .get(&caller.session.principal, connector_id(&id)?)
        .await?;
    Ok(Json(ConnectorBody::from(&found)))
}

pub async fn update_connector<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateConnectorBody>,
) -> Result<Json<ConnectorBody>, ApiError> {
    let choices = choices(
        &body.kind,
        &body.settings,
        body.secret_name.as_deref(),
        body.default_event_type.as_deref(),
        body.rate_per_second,
        body.status.as_deref(),
    )?;
    let updated = connectors(&state)?
        .update(
            &caller.session.principal,
            connector_id(&id)?,
            choices,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ConnectorBody::from(&updated)))
}

pub async fn delete_connector<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    connectors(&state)?
        .delete(&caller.session.principal, connector_id(&id)?, &caller.ctx)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_dead_letters<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Json<DeadLetterPageBody>, ApiError> {
    let before = query
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<Uuid>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut items = connectors(&state)?
        .dead_letters(
            &caller.session.principal,
            connector_id(&id)?,
            before,
            limit + 1,
        )
        .await?;
    let next_cursor = if items.len() > limit as usize {
        items.truncate(limit as usize);
        items.last().map(|d| d.id.to_string())
    } else {
        None
    };
    Ok(Json(DeadLetterPageBody {
        items: items.iter().map(DeadLetterBody::from).collect(),
        next_cursor,
    }))
}

pub async fn get_dead_letter<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id, letter)): Path<(String, String, String)>,
) -> Result<Json<DeadLetterBody>, ApiError> {
    let found = connectors(&state)?
        .dead_letter(
            &caller.session.principal,
            connector_id(&id)?,
            letter_id(&letter)?,
        )
        .await?;
    Ok(Json(DeadLetterBody::from(&found)))
}

pub async fn discard_dead_letter<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id, letter)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    connectors(&state)?
        .discard(
            &caller.session.principal,
            connector_id(&id)?,
            letter_id(&letter)?,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn redrive_dead_letter<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((_namespace, id, letter)): Path<(String, String, String)>,
) -> Result<Json<AcceptedBody>, ApiError> {
    let key = connectors(&state)?
        .redrive(
            &caller.session.principal,
            connector_id(&id)?,
            letter_id(&letter)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(AcceptedBody {
        event_id: key.event_id.to_string(),
        source: key.source.to_string(),
        status: "ACCEPTED",
    }))
}

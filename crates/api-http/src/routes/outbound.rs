//! Subscriptions to outbound events, and their deliveries.
//!
//! A subscription is addressed through its namespace, and one that belongs to another tenant is a
//! `404` like one that does not exist.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::outbound::{BrokerSubscriptionSettings, SubscriptionSettings};
use sagat_domain::connectors::ConnectorKind;
use sagat_domain::identity::{NamespaceId, Slug};
use sagat_domain::outbound::{
    BrokerDestination, DeliveryId, DeliveryLimits, DeliveryStatus, OutboundKind, SubscriptionId,
    SubscriptionStatus, TargetUrl,
};
use sagat_domain::secrets::SecretName;
use serde::Deserialize;

use crate::admission::admit;
use crate::dto_events::{
    BrokerSubscriptionBody, BrokerSubscriptionListBody, CreateBrokerSubscriptionBody,
    CreateSubscriptionBody, DeliveryBody, DeliveryPageBody, OutboundKafkaSettingsBody,
    OutboundNatsSettingsBody, OutboundSqsSettingsBody, RedriveBody, RedrivenBody, SubscriptionBody,
    SubscriptionListBody, UpdateBrokerSubscriptionBody, UpdateSubscriptionBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn subscription_id(raw: &str) -> Result<SubscriptionId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn delivery_id(raw: &str) -> Result<DeliveryId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn outbound<B: Governance>(
    state: &B,
) -> Result<&sagat_application::outbound::OutboundService<B::Store, B::Clock>, ApiError> {
    state.outbound().ok_or_else(ApiError::not_found)
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

async fn admitted_subscription<B: Governance>(
    state: &B,
    caller: &Caller,
    raw_namespace: &str,
    raw_subscription: &str,
    broker: bool,
    operation: ApiOperation,
) -> Result<sagat_application::outbound::SubscriptionRecord, ApiError> {
    let namespace = namespace_id(raw_namespace)?;
    let id = subscription_id(raw_subscription)?;
    let subscription = if broker {
        outbound(state)?
            .get_broker(&caller.session.principal, id)
            .await?
    } else {
        outbound(state)?.get(&caller.session.principal, id).await?
    };
    if subscription.namespace_id != namespace {
        return Err(ApiError::not_found());
    }
    admit(
        state,
        caller,
        operation,
        ApiTarget::Namespace(subscription.namespace_id),
    )
    .await?;
    Ok(subscription)
}

fn settings(
    url: &str,
    secret_name: &str,
    events: &[String],
    rate_per_second: Option<u32>,
    timeout_ms: Option<u64>,
    status: Option<&str>,
) -> Result<SubscriptionSettings, ApiError> {
    let kinds = events
        .iter()
        .map(|e| {
            e.parse::<OutboundKind>().map_err(|()| {
                ApiError::validation("an event type is not one that can be delivered")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let limits = DeliveryLimits::new(
        rate_per_second.unwrap_or(DeliveryLimits::DEFAULT_RATE),
        timeout_ms.map_or(DeliveryLimits::DEFAULT_TIMEOUT, Duration::from_millis),
    )
    .map_err(|e| ApiError::validation(e.to_string()))?;
    Ok(SubscriptionSettings {
        url: TargetUrl::parse(url).map_err(|e| ApiError::validation(e.to_string()))?,
        secret_name: SecretName::parse(secret_name)
            .map_err(|_| ApiError::validation("the secret name is not valid"))?,
        kinds,
        limits,
        status: match status {
            None => SubscriptionStatus::Active,
            Some(text) => text
                .parse()
                .map_err(|()| ApiError::validation("the status is ACTIVE or PAUSED"))?,
        },
    })
}

/// Whether the caller sees the whole URL of a subscription. Its path and query can be the
/// credential of the receiver (an incoming-webhook URL of a chat tool), so only those who manage
/// events see them; a reader sees where it goes.
fn can_see_urls(caller: &Caller) -> bool {
    caller
        .session
        .principal
        .authorize(sagat_domain::identity::Permission::EventManage)
        .is_ok()
}

pub async fn create_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<CreateSubscriptionBody>,
) -> Result<(StatusCode, Json<SubscriptionBody>), ApiError> {
    let settings = settings(
        &body.url,
        &body.secret_name,
        &body.events,
        body.rate_per_second,
        body.timeout_ms,
        body.status.as_deref(),
    )?;
    let name =
        Slug::parse(&body.name).map_err(|_| ApiError::validation("the name is not valid"))?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventManage).await?;
    let created = outbound(&state)?
        .create(
            &caller.session.principal,
            namespace,
            name,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(SubscriptionBody::from(&created))))
}

pub async fn list_subscriptions<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<SubscriptionListBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventRead).await?;
    let items = outbound(&state)?
        .list(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    let reveal = can_see_urls(&caller);
    Ok(Json(SubscriptionListBody {
        items: items
            .iter()
            .map(|item| SubscriptionBody::of(item, reveal))
            .collect(),
    }))
}

pub async fn get_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<SubscriptionBody>, ApiError> {
    let found = admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventRead,
    )
    .await?;
    Ok(Json(SubscriptionBody::of(&found, can_see_urls(&caller))))
}

pub async fn update_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateSubscriptionBody>,
) -> Result<Json<SubscriptionBody>, ApiError> {
    let settings = settings(
        &body.url,
        &body.secret_name,
        &body.events,
        body.rate_per_second,
        body.timeout_ms,
        body.status.as_deref(),
    )?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventManage,
    )
    .await?;
    let updated = outbound(&state)?
        .update(
            &caller.session.principal,
            subscription_id(&id)?,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok(Json(SubscriptionBody::from(&updated)))
}

pub async fn delete_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventManage,
    )
    .await?;
    outbound(&state)?
        .delete(
            &caller.session.principal,
            subscription_id(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
pub struct DeliveryQuery {
    pub status: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_deliveries<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<DeliveryPageBody>, ApiError> {
    let status = query
        .status
        .as_deref()
        .map(|s| {
            s.parse::<DeliveryStatus>()
                .map_err(|()| ApiError::bad_request("status is PENDING, DELIVERED or DEAD"))
        })
        .transpose()?;
    let before = query
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<DeliveryId>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventRead,
    )
    .await?;
    let mut items = outbound(&state)?
        .deliveries(
            &caller.session.principal,
            subscription_id(&id)?,
            status,
            before,
            limit + 1,
        )
        .await?;
    // One more than asked for tells whether there is a next page.
    let next_cursor = if items.len() > limit as usize {
        items.truncate(limit as usize);
        items.last().map(|d| d.id.to_string())
    } else {
        None
    };
    Ok(Json(DeliveryPageBody {
        items: items.iter().map(DeliveryBody::listed).collect(),
        next_cursor,
    }))
}

pub async fn get_delivery<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, delivery)): Path<(String, String, String)>,
) -> Result<Json<DeliveryBody>, ApiError> {
    let delivery = delivery_id(&delivery)?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventRead,
    )
    .await?;
    let (delivery, attempts) = outbound(&state)?
        .delivery(&caller.session.principal, subscription_id(&id)?, delivery)
        .await?;
    Ok(Json(DeliveryBody::detailed(&delivery, &attempts)))
}

pub async fn redrive_delivery<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, delivery)): Path<(String, String, String)>,
) -> Result<Json<RedrivenBody>, ApiError> {
    let delivery = delivery_id(&delivery)?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventManage,
    )
    .await?;
    let redriven = outbound(&state)?
        .redrive(
            &caller.session.principal,
            subscription_id(&id)?,
            Some(delivery),
            1,
            &caller.ctx,
        )
        .await?;
    if redriven == 0 {
        // Not dead (or not there): there was nothing to put back.
        return Err(
            sagat_application::error::AppError::StateConflict("the delivery is not dead").into(),
        );
    }
    Ok(Json(RedrivenBody { redriven }))
}

pub async fn redrive_dead<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<RedriveBody>,
) -> Result<Json<RedrivenBody>, ApiError> {
    let limit = body.limit.unwrap_or(100);
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        false,
        ApiOperation::EventManage,
    )
    .await?;
    let redriven = outbound(&state)?
        .redrive(
            &caller.session.principal,
            subscription_id(&id)?,
            None,
            limit,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RedrivenBody { redriven }))
}

fn broker_settings(
    kind: &str,
    value: &serde_json::Value,
    secret_name: Option<&str>,
    events: &[String],
    rate_per_second: Option<u32>,
    timeout_ms: Option<u64>,
    status: Option<&str>,
) -> Result<BrokerSubscriptionSettings, ApiError> {
    let kind = kind
        .parse::<ConnectorKind>()
        .map_err(|()| ApiError::validation("the kind is NATS, KAFKA or SQS"))?;
    let invalid = |_| ApiError::validation("the settings do not fit the broker kind");
    let refused = |e: sagat_domain::connectors::ConnectorError| ApiError::validation(e.to_string());
    let destination = match kind {
        ConnectorKind::Nats => {
            let s: OutboundNatsSettingsBody =
                serde_json::from_value(value.clone()).map_err(invalid)?;
            BrokerDestination::nats(&s.servers, &s.subject).map_err(refused)?
        }
        ConnectorKind::Kafka => {
            let s: OutboundKafkaSettingsBody =
                serde_json::from_value(value.clone()).map_err(invalid)?;
            BrokerDestination::kafka(&s.servers, &s.topic).map_err(refused)?
        }
        ConnectorKind::Sqs => {
            let s: OutboundSqsSettingsBody =
                serde_json::from_value(value.clone()).map_err(invalid)?;
            BrokerDestination::sqs(&s.queue_url, &s.region).map_err(refused)?
        }
    };
    let kinds = events
        .iter()
        .map(|e| {
            e.parse::<OutboundKind>().map_err(|()| {
                ApiError::validation("an event type is not one that can be delivered")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BrokerSubscriptionSettings {
        destination,
        credentials_secret: secret_name
            .map(|s| {
                SecretName::parse(s)
                    .map_err(|_| ApiError::validation("the secret name is not valid"))
            })
            .transpose()?,
        kinds,
        limits: DeliveryLimits::new(
            rate_per_second.unwrap_or(DeliveryLimits::DEFAULT_RATE),
            timeout_ms.map_or(DeliveryLimits::DEFAULT_TIMEOUT, Duration::from_millis),
        )
        .map_err(|e| ApiError::validation(e.to_string()))?,
        status: match status {
            None => SubscriptionStatus::Active,
            Some(text) => text
                .parse()
                .map_err(|()| ApiError::validation("the status is ACTIVE or PAUSED"))?,
        },
    })
}

pub async fn create_broker_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<CreateBrokerSubscriptionBody>,
) -> Result<(StatusCode, Json<BrokerSubscriptionBody>), ApiError> {
    let settings = broker_settings(
        &body.kind,
        &body.settings,
        body.secret_name.as_deref(),
        &body.events,
        body.rate_per_second,
        body.timeout_ms,
        body.status.as_deref(),
    )?;
    let name =
        Slug::parse(&body.name).map_err(|_| ApiError::validation("the name is not valid"))?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventManage).await?;
    let created = outbound(&state)?
        .create_broker(
            &caller.session.principal,
            namespace,
            name,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(BrokerSubscriptionBody::from(&created)),
    ))
}

pub async fn list_broker_subscriptions<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<BrokerSubscriptionListBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EventRead).await?;
    let items = outbound(&state)?
        .list_brokers(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(BrokerSubscriptionListBody {
        items: items.iter().map(BrokerSubscriptionBody::from).collect(),
    }))
}

pub async fn get_broker_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<BrokerSubscriptionBody>, ApiError> {
    let found = admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventRead,
    )
    .await?;
    Ok(Json(BrokerSubscriptionBody::from(&found)))
}

pub async fn update_broker_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateBrokerSubscriptionBody>,
) -> Result<Json<BrokerSubscriptionBody>, ApiError> {
    let settings = broker_settings(
        &body.kind,
        &body.settings,
        body.secret_name.as_deref(),
        &body.events,
        body.rate_per_second,
        body.timeout_ms,
        body.status.as_deref(),
    )?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventManage,
    )
    .await?;
    let updated = outbound(&state)?
        .update_broker(
            &caller.session.principal,
            subscription_id(&id)?,
            settings,
            &caller.ctx,
        )
        .await?;
    Ok(Json(BrokerSubscriptionBody::from(&updated)))
}

pub async fn delete_broker_subscription<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventManage,
    )
    .await?;
    outbound(&state)?
        .delete_broker(
            &caller.session.principal,
            subscription_id(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_broker_deliveries<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<DeliveryPageBody>, ApiError> {
    let status = query
        .status
        .as_deref()
        .map(|s| {
            s.parse::<DeliveryStatus>()
                .map_err(|()| ApiError::bad_request("status is PENDING, DELIVERED or DEAD"))
        })
        .transpose()?;
    let before = query
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<DeliveryId>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventRead,
    )
    .await?;
    let mut items = outbound(&state)?
        .broker_deliveries(
            &caller.session.principal,
            subscription_id(&id)?,
            status,
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
    Ok(Json(DeliveryPageBody {
        items: items.iter().map(DeliveryBody::listed).collect(),
        next_cursor,
    }))
}

pub async fn get_broker_delivery<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, delivery)): Path<(String, String, String)>,
) -> Result<Json<DeliveryBody>, ApiError> {
    let delivery = delivery_id(&delivery)?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventRead,
    )
    .await?;
    let (delivery, attempts) = outbound(&state)?
        .broker_delivery(&caller.session.principal, subscription_id(&id)?, delivery)
        .await?;
    Ok(Json(DeliveryBody::detailed(&delivery, &attempts)))
}

pub async fn redrive_broker_delivery<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, delivery)): Path<(String, String, String)>,
) -> Result<Json<RedrivenBody>, ApiError> {
    let delivery = delivery_id(&delivery)?;
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventManage,
    )
    .await?;
    let redriven = outbound(&state)?
        .redrive_broker(
            &caller.session.principal,
            subscription_id(&id)?,
            Some(delivery),
            1,
            &caller.ctx,
        )
        .await?;
    if redriven == 0 {
        return Err(
            sagat_application::error::AppError::StateConflict("the delivery is not dead").into(),
        );
    }
    Ok(Json(RedrivenBody { redriven }))
}

pub async fn redrive_broker_dead<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<RedriveBody>,
) -> Result<Json<RedrivenBody>, ApiError> {
    admitted_subscription(
        &state,
        &caller,
        &namespace,
        &id,
        true,
        ApiOperation::EventManage,
    )
    .await?;
    let redriven = outbound(&state)?
        .redrive_broker(
            &caller.session.principal,
            subscription_id(&id)?,
            None,
            body.limit.unwrap_or(100),
            &caller.ctx,
        )
        .await?;
    Ok(Json(RedrivenBody { redriven }))
}

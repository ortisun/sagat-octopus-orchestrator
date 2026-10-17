//! Wire types of the event routes.

use sagat_application::event_payloads::{
    DeadLetterData, DeadLetterRead, DeadLetterStructure, EventData, InboxEventRead,
};
use sagat_application::events::{Delivery, EndpointRecord, Received};
use sagat_application::payloads::PayloadStructure;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where the public URL of an endpoint is, relative to the host the API is served from.
fn webhook_path(id: impl std::fmt::Display) -> String {
    format!("/v1/webhooks/{id}")
}

/// What an administrator chooses about an endpoint. Every setting but the secret has a default.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateEndpointBody {
    pub slug: String,
    pub secret_name: Option<String>,
    #[serde(default)]
    pub authentication: Option<WebhookAuthenticationBody>,
    #[serde(default)]
    pub default_event_type: Option<String>,
    #[serde(default)]
    pub dedupe_ttl_seconds: Option<u64>,
    #[serde(default)]
    pub skew_seconds: Option<u64>,
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    /// `ACTIVE` (the default) or `PAUSED`.
    #[serde(default)]
    pub status: Option<String>,
}

/// The settings of an existing endpoint, replaced as a whole: the slug and the id never change.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateEndpointBody {
    pub secret_name: Option<String>,
    #[serde(default)]
    pub authentication: Option<WebhookAuthenticationBody>,
    #[serde(default)]
    pub default_event_type: Option<String>,
    #[serde(default)]
    pub dedupe_ttl_seconds: Option<u64>,
    #[serde(default)]
    pub skew_seconds: Option<u64>,
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointBody {
    pub id: String,
    pub namespace_id: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    pub authentication: WebhookAuthenticationBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_event_type: Option<String>,
    pub dedupe_ttl_seconds: u64,
    pub skew_seconds: u64,
    pub max_body_bytes: usize,
    pub rate_per_second: u32,
    pub status: &'static str,
    /// The path providers post to. Anybody who knows it can try; only a signed request is taken.
    pub webhook_path: String,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

impl From<&EndpointRecord> for EndpointBody {
    fn from(endpoint: &EndpointRecord) -> Self {
        Self {
            id: endpoint.id.to_string(),
            namespace_id: endpoint.namespace_id.to_string(),
            slug: endpoint.slug.to_string(),
            secret_name: endpoint.secret_name.as_ref().map(ToString::to_string),
            authentication: WebhookAuthenticationBody::from(&endpoint.authentication),
            default_event_type: endpoint
                .default_event_type
                .as_ref()
                .map(ToString::to_string),
            dedupe_ttl_seconds: endpoint.limits.dedupe_ttl().as_secs(),
            skew_seconds: endpoint.limits.skew().as_secs(),
            max_body_bytes: endpoint.limits.max_body(),
            rate_per_second: endpoint.limits.rate_per_second(),
            status: endpoint.status.as_str(),
            webhook_path: webhook_path(endpoint.id),
            created_by: endpoint.created_by.clone(),
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EndpointListBody {
    pub items: Vec<EndpointBody>,
}

/// An event published through the API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishEventBody {
    /// Absent: the platform makes one up, and the event can then not be a duplicate of another.
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub occurred_at: Option<time::OffsetDateTime>,
    #[serde(default)]
    pub correlation_key: Option<String>,
    pub data: Value,
    #[serde(default)]
    pub dedupe_ttl_seconds: Option<u64>,
}

/// The answer to a request that was understood: the event is in the inbox, now or since before.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedBody {
    pub event_id: String,
    pub source: String,
    /// `ACCEPTED`, or `DUPLICATE` when the same event had been admitted before (nothing was done).
    pub status: &'static str,
}

impl From<&Received> for AcceptedBody {
    fn from(received: &Received) -> Self {
        Self {
            event_id: received.key.event_id.to_string(),
            source: received.key.source.to_string(),
            status: if received.duplicate {
                "DUPLICATE"
            } else {
                "ACCEPTED"
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDeliveryBody {
    pub consumer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub delivered_at: time::OffsetDateTime,
}

impl From<&Delivery> for EventDeliveryBody {
    fn from(delivery: &Delivery) -> Self {
        Self {
            consumer: delivery.consumer.clone(),
            run_id: delivery.run_id.map(|id| id.to_string()),
            node: delivery.node.clone(),
            trigger_id: delivery.trigger_id.map(|id| id.to_string()),
            delivered_at: delivery.delivered_at,
        }
    }
}

/// An event of the inbox, with what it woke or started.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventBody {
    pub event_id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_key: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub occurred_at: Option<time::OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: time::OffsetDateTime,
    /// The end of the deduplication window.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: time::OffsetDateTime,
    /// `PENDING`, `DISPATCHED` or `DEAD`.
    pub status: &'static str,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub dispatched_at: Option<time::OffsetDateTime>,
    pub data_disclosure: &'static str,
    pub data_size_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_structure: Option<PayloadStructureBody>,
    pub deliveries: Vec<EventDeliveryBody>,
}

impl From<&InboxEventRead> for EventBody {
    fn from(read: &InboxEventRead) -> Self {
        let event = &read.event;
        let (data_disclosure, data, data_structure) = match &read.data {
            EventData::Revealed(bytes) => (
                "revealed",
                serde_json::from_slice(bytes.as_slice()).ok(),
                None,
            ),
            EventData::Structure(projection) => (
                "structure-only",
                None,
                Some(PayloadStructureBody::from(&projection.tree)),
            ),
        };
        Self {
            event_id: event.envelope.key.event_id.to_string(),
            source: event.envelope.key.source.to_string(),
            event_type: event.envelope.event_type.to_string(),
            subject: event.envelope.subject.clone(),
            correlation_key: event
                .envelope
                .correlation_key
                .as_ref()
                .map(|k| k.as_str().to_owned()),
            occurred_at: event.occurred_at,
            received_at: event.received_at,
            expires_at: event.expires_at,
            status: event.status.as_str(),
            attempts: event.attempts,
            last_error: event.last_error.clone(),
            dispatched_at: event.dispatched_at,
            data_disclosure,
            data_size_bytes: read.data_size_bytes,
            data,
            data_structure,
            deliveries: event
                .deliveries
                .iter()
                .map(EventDeliveryBody::from)
                .collect(),
        }
    }
}

// ------------------------------------------------------------------------------ outbound events

use sagat_application::outbound::{AttemptRecord, DeliveryRecord, SubscriptionRecord};

/// What an administrator chooses about a subscription. Every setting but the first three has a
/// default.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSubscriptionBody {
    pub name: String,
    pub url: String,
    pub secret_name: String,
    /// The kinds of event to deliver, such as `run.finished`.
    pub events: Vec<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// `ACTIVE` (the default) or `PAUSED`.
    #[serde(default)]
    pub status: Option<String>,
}

/// The settings of an existing subscription, replaced as a whole: its name and id never change.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSubscriptionBody {
    pub url: String,
    pub secret_name: String,
    pub events: Vec<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthBody {
    pub consecutive_dead: u32,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub last_success_at: Option<time::OffsetDateTime>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub last_failure_at: Option<time::OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionBody {
    pub id: String,
    pub namespace_id: String,
    pub name: String,
    pub url: String,
    pub secret_name: String,
    pub events: Vec<&'static str>,
    pub rate_per_second: u32,
    pub timeout_ms: u64,
    /// `ACTIVE`, `PAUSED`, or `DISABLED` (switched off by the platform after too many deliveries
    /// were set aside; turn it back on by setting it `ACTIVE`).
    pub status: &'static str,
    pub health: HealthBody,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

/// Where a subscription goes, without the part that can be a credential: chat tools and many
/// receivers hand out URLs whose path or query is the secret. What is kept is the scheme, the host
/// and a port that is not the scheme's default, which is what a reader needs to know.
fn redacted_url(url: &sagat_domain::outbound::TargetUrl) -> String {
    let destination = url.destination();
    let default = if destination.scheme == "https" {
        443
    } else {
        80
    };
    if destination.port == default {
        format!("{}://{}/…", destination.scheme, destination.host)
    } else {
        format!(
            "{}://{}:{}/…",
            destination.scheme, destination.host, destination.port
        )
    }
}

impl SubscriptionBody {
    /// The subscription as `reveal`ing callers (those who can manage events) and others see it.
    #[must_use]
    pub fn of(subscription: &SubscriptionRecord, reveal: bool) -> Self {
        let sagat_domain::outbound::OutboundDestination::Webhook { url, .. } =
            &subscription.destination
        else {
            unreachable!("webhook routes return webhook subscriptions")
        };
        Self {
            url: if reveal {
                url.as_str().to_owned()
            } else {
                redacted_url(url)
            },
            ..Self::from(subscription)
        }
    }
}

impl From<&SubscriptionRecord> for SubscriptionBody {
    fn from(subscription: &SubscriptionRecord) -> Self {
        let sagat_domain::outbound::OutboundDestination::Webhook {
            url,
            signing_secret,
        } = &subscription.destination
        else {
            unreachable!("webhook routes return webhook subscriptions")
        };
        Self {
            id: subscription.id.to_string(),
            namespace_id: subscription.namespace_id.to_string(),
            name: subscription.name.to_string(),
            url: url.as_str().to_owned(),
            secret_name: signing_secret.to_string(),
            events: subscription.kinds.iter().map(|k| k.as_str()).collect(),
            rate_per_second: subscription.limits.rate_per_second(),
            timeout_ms: u64::try_from(subscription.limits.timeout().as_millis()).unwrap_or(0),
            status: subscription.status.as_str(),
            health: HealthBody {
                consecutive_dead: subscription.health.consecutive_dead,
                last_success_at: subscription.health.last_success_at,
                last_failure_at: subscription.health.last_failure_at,
                disabled_reason: subscription.health.disabled_reason.clone(),
            },
            created_by: subscription.created_by.clone(),
            created_at: subscription.created_at,
            updated_at: subscription.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SubscriptionListBody {
    pub items: Vec<SubscriptionBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBrokerSubscriptionBody {
    pub name: String,
    pub kind: String,
    pub settings: Value,
    #[serde(default)]
    pub secret_name: Option<String>,
    pub events: Vec<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateBrokerSubscriptionBody {
    pub kind: String,
    pub settings: Value,
    #[serde(default)]
    pub secret_name: Option<String>,
    pub events: Vec<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutboundNatsSettingsBody {
    pub servers: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutboundKafkaSettingsBody {
    pub servers: Vec<String>,
    pub topic: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutboundSqsSettingsBody {
    pub queue_url: String,
    pub region: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSubscriptionBody {
    pub id: String,
    pub namespace_id: String,
    pub name: String,
    pub kind: &'static str,
    pub settings: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    pub events: Vec<&'static str>,
    pub rate_per_second: u32,
    pub timeout_ms: u64,
    pub status: &'static str,
    pub health: HealthBody,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

impl From<&SubscriptionRecord> for BrokerSubscriptionBody {
    fn from(subscription: &SubscriptionRecord) -> Self {
        use sagat_domain::outbound::{BrokerDestination, OutboundDestination};
        let OutboundDestination::Broker {
            settings,
            credentials_secret,
        } = &subscription.destination
        else {
            unreachable!("broker routes return broker subscriptions")
        };
        let value = match settings {
            BrokerDestination::Nats { servers, subject } => serde_json::json!({
                "servers": servers.iter().map(sagat_domain::connectors::Server::configured_address).collect::<Vec<_>>(),
                "subject": subject,
            }),
            BrokerDestination::Kafka { servers, topic } => serde_json::json!({
                "servers": servers.iter().map(sagat_domain::connectors::Server::configured_address).collect::<Vec<_>>(),
                "topic": topic,
            }),
            BrokerDestination::Sqs { queue_url, region } => serde_json::json!({
                "queueUrl": queue_url.as_str(),
                "region": region,
            }),
        };
        Self {
            id: subscription.id.to_string(),
            namespace_id: subscription.namespace_id.to_string(),
            name: subscription.name.to_string(),
            kind: settings.kind().as_str(),
            settings: value,
            secret_name: credentials_secret.as_ref().map(ToString::to_string),
            events: subscription.kinds.iter().map(|k| k.as_str()).collect(),
            rate_per_second: subscription.limits.rate_per_second(),
            timeout_ms: u64::try_from(subscription.limits.timeout().as_millis()).unwrap_or(0),
            status: subscription.status.as_str(),
            health: HealthBody {
                consecutive_dead: subscription.health.consecutive_dead,
                last_success_at: subscription.health.last_success_at,
                last_failure_at: subscription.health.last_failure_at,
                disabled_reason: subscription.health.disabled_reason.clone(),
            },
            created_by: subscription.created_by.clone(),
            created_at: subscription.created_at,
            updated_at: subscription.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BrokerSubscriptionListBody {
    pub items: Vec<BrokerSubscriptionBody>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptBody {
    pub attempt: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: time::OffsetDateTime,
    pub duration_ms: u32,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&AttemptRecord> for AttemptBody {
    fn from(attempt: &AttemptRecord) -> Self {
        Self {
            attempt: attempt.attempt,
            started_at: attempt.started_at,
            duration_ms: attempt.duration_ms,
            outcome: attempt.outcome.as_str(),
            http_status: attempt.http_status,
            error: attempt.error.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryBody {
    pub id: String,
    pub subscription_id: String,
    /// The id of the event (the same for every subscription it is delivered to).
    pub event_id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub status: &'static str,
    pub attempts: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub next_attempt_at: time::OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub delivered_at: Option<time::OffsetDateTime>,
    /// What is sent (and signed), only when one delivery is asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Every attempt made, oldest first, only when one delivery is asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_made: Option<Vec<AttemptBody>>,
}

impl DeliveryBody {
    #[must_use]
    pub fn listed(delivery: &DeliveryRecord) -> Self {
        Self {
            id: delivery.id.to_string(),
            subscription_id: delivery.subscription_id.to_string(),
            event_id: delivery.event_id.clone(),
            kind: delivery.kind.as_str(),
            status: delivery.status.as_str(),
            attempts: delivery.attempts,
            next_attempt_at: delivery.next_attempt_at,
            last_outcome: delivery.last_outcome.map(|o| o.as_str()),
            last_http_status: delivery.last_http_status,
            last_error: delivery.last_error.clone(),
            created_at: delivery.created_at,
            delivered_at: delivery.delivered_at,
            payload: None,
            attempts_made: None,
        }
    }

    #[must_use]
    pub fn detailed(delivery: &DeliveryRecord, attempts: &[AttemptRecord]) -> Self {
        Self {
            payload: Some(serde_json::from_slice(&delivery.payload).unwrap_or(Value::Null)),
            attempts_made: Some(attempts.iter().map(AttemptBody::from).collect()),
            ..Self::listed(delivery)
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryPageBody {
    pub items: Vec<DeliveryBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// How many dead deliveries to put back, at most.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedriveBody {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct RedrivenBody {
    pub redriven: u64,
}

// ------------------------------------------------------------------------------ connectors

use sagat_application::connectors::ConnectorRecord;
use sagat_domain::connectors::{ConnectorSettings, StartFrom};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NatsSettingsBody {
    pub servers: Vec<String>,
    pub stream: String,
    pub consumer: String,
    #[serde(default)]
    pub filter_subject: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KafkaSettingsBody {
    pub servers: Vec<String>,
    pub topic: String,
    /// `earliest` (the default) or `latest`.
    #[serde(default)]
    pub start_from: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqsSettingsBody {
    pub queue_url: String,
    pub region: String,
}

/// What a caller says about a connector. `settings` is shaped by `kind`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConnectorBody {
    pub slug: String,
    pub kind: String,
    pub settings: Value,
    #[serde(default)]
    pub secret_name: Option<String>,
    #[serde(default)]
    pub default_event_type: Option<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConnectorBody {
    pub kind: String,
    pub settings: Value,
    #[serde(default)]
    pub secret_name: Option<String>,
    #[serde(default)]
    pub default_event_type: Option<String>,
    #[serde(default)]
    pub rate_per_second: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
}

/// The settings as they are shown.
fn settings_value(settings: &ConnectorSettings) -> Value {
    let servers = |servers: &[sagat_domain::connectors::Server]| -> Vec<String> {
        servers
            .iter()
            .map(|s| format!("{}{}", if s.tls() { "tls://" } else { "" }, s.address()))
            .collect()
    };
    match settings {
        ConnectorSettings::Nats {
            servers: s,
            stream,
            consumer,
            filter_subject,
        } => serde_json::json!({
            "servers": servers(s), "stream": stream, "consumer": consumer, "filterSubject": filter_subject
        }),
        ConnectorSettings::Kafka {
            servers: s,
            topic,
            start_from,
        } => serde_json::json!({
            "servers": servers(s), "topic": topic,
            "startFrom": match start_from { StartFrom::Earliest => "earliest", StartFrom::Latest => "latest" },
        }),
        ConnectorSettings::Sqs { queue_url, region } => {
            serde_json::json!({ "queueUrl": queue_url, "region": region })
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorHealthBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub last_message_at: Option<time::OffsetDateTime>,
    pub consumed: u64,
    pub dead_lettered: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorBody {
    pub id: String,
    pub namespace_id: String,
    pub slug: String,
    pub kind: &'static str,
    pub settings: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_event_type: Option<String>,
    pub rate_per_second: u32,
    pub status: &'static str,
    pub health: ConnectorHealthBody,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

impl From<&ConnectorRecord> for ConnectorBody {
    fn from(connector: &ConnectorRecord) -> Self {
        Self {
            id: connector.id.to_string(),
            namespace_id: connector.namespace_id.to_string(),
            slug: connector.slug.to_string(),
            kind: connector.settings.kind().as_str(),
            settings: settings_value(&connector.settings),
            secret_name: connector.secret_name.as_ref().map(ToString::to_string),
            default_event_type: connector
                .default_event_type
                .as_ref()
                .map(ToString::to_string),
            rate_per_second: connector.rate_per_second,
            status: connector.status.as_str(),
            health: ConnectorHealthBody {
                last_error: connector.health.last_error.clone(),
                last_message_at: connector.health.last_message_at,
                consumed: connector.health.consumed,
                dead_lettered: connector.health.dead_lettered,
            },
            created_by: connector.created_by.clone(),
            created_at: connector.created_at,
            updated_at: connector.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectorListBody {
    pub items: Vec<ConnectorBody>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetterBody {
    pub id: String,
    pub position: String,
    pub reason: &'static str,
    pub body_disclosure: &'static str,
    pub body_size_bytes: usize,
    pub header_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    /// What the message carried, as text (cut at 64 KiB: see `truncated`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_structure: Option<PayloadStructureBody>,
    pub truncated: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: time::OffsetDateTime,
}

impl From<&DeadLetterRead> for DeadLetterBody {
    fn from(read: &DeadLetterRead) -> Self {
        let letter = &read.letter;
        let (body_disclosure, headers, body, body_structure) = match &read.data {
            DeadLetterData::Revealed { headers, body } => (
                "revealed",
                Some(headers.clone()),
                Some(String::from_utf8_lossy(body.as_slice()).into_owned()),
                None,
            ),
            DeadLetterData::Structure(structure) => (
                "structure-only",
                None,
                None,
                Some(match structure {
                    DeadLetterStructure::Json(projection) => {
                        PayloadStructureBody::from(&projection.tree)
                    }
                    DeadLetterStructure::Opaque => PayloadStructureBody::Opaque { kind: "opaque" },
                }),
            ),
        };
        Self {
            id: letter.id.to_string(),
            position: letter.position.clone(),
            reason: letter.reason.as_str(),
            body_disclosure,
            body_size_bytes: read.body_size_bytes,
            header_count: read.header_count,
            headers,
            body,
            body_structure,
            truncated: letter.truncated,
            received_at: letter.received_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum PayloadStructureBody {
    Structure(PayloadTreeBody),
    Opaque { kind: &'static str },
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum PayloadTreeBody {
    Object {
        child_count: u32,
        children: Vec<PayloadStructureBody>,
        omitted_children: u32,
    },
    Array {
        child_count: u32,
        children: Vec<PayloadStructureBody>,
        omitted_children: u32,
    },
    String {
        redacted: bool,
    },
    Number {
        redacted: bool,
    },
    Boolean {
        redacted: bool,
    },
    Null {
        redacted: bool,
    },
}

impl From<&PayloadStructure> for PayloadStructureBody {
    fn from(value: &PayloadStructure) -> Self {
        Self::Structure(match value {
            PayloadStructure::Object {
                child_count,
                children,
                omitted_children,
            } => PayloadTreeBody::Object {
                child_count: *child_count,
                children: children.iter().map(Self::from).collect(),
                omitted_children: *omitted_children,
            },
            PayloadStructure::Array {
                child_count,
                children,
                omitted_children,
            } => PayloadTreeBody::Array {
                child_count: *child_count,
                children: children.iter().map(Self::from).collect(),
                omitted_children: *omitted_children,
            },
            PayloadStructure::String => PayloadTreeBody::String { redacted: true },
            PayloadStructure::Number => PayloadTreeBody::Number { redacted: true },
            PayloadStructure::Boolean => PayloadTreeBody::Boolean { redacted: true },
            PayloadStructure::Null => PayloadTreeBody::Null { redacted: true },
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetterPageBody {
    pub items: Vec<DeadLetterBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Exactly one credential mechanism, with no client-controlled trust roots or discovery URLs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WebhookAuthenticationBody {
    #[serde(rename = "HMAC")]
    Hmac,
    #[serde(rename = "OIDC")]
    Oidc { profile: String, subject: String },
    #[serde(rename = "MTLS")]
    Mtls {
        #[serde(rename = "certificateSha256")]
        certificate_sha256: String,
    },
}

impl From<&sagat_domain::events::WebhookAuthentication> for WebhookAuthenticationBody {
    fn from(value: &sagat_domain::events::WebhookAuthentication) -> Self {
        use sagat_domain::events::WebhookAuthentication;
        match value {
            WebhookAuthentication::Hmac => Self::Hmac,
            WebhookAuthentication::Oidc { profile, subject } => Self::Oidc {
                profile: profile.to_string(),
                subject: subject.clone(),
            },
            WebhookAuthentication::Mtls { certificate_sha256 } => Self::Mtls {
                certificate_sha256: certificate_sha256.clone(),
            },
        }
    }
}

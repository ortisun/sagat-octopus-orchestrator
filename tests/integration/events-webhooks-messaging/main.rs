//! Feature 004 integration suite against a real PostgreSQL: events admitted once and only when they
//! verify, matched to triggers and waiting steps without loss or duplication, and delivered out.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod connectors;
mod cron;
mod dispatch;
mod ingress;
mod outbound;
mod sender;

use sagat_application::events::{EndpointRecord, EndpointSettings, WebhookRequest};
use sagat_domain::events::EndpointLimits;
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use support::engine::{World, ctx};

/// What a provider shares with the platform.
pub const SECRET: &str = "whsec-0123456789abcdef0123456789abcdef";
pub const SECRET_NAME: &str = "provider-signing";

pub fn settings() -> EndpointSettings {
    EndpointSettings {
        secret_name: SecretName::parse(SECRET_NAME).expect("secret name"),
        default_event_type: None,
        limits: EndpointLimits::default(),
        status: sagat_domain::events::EndpointStatus::Active,
    }
}

/// A world with the signing secret stored and one endpoint, `billing`, ready to receive.
pub async fn with_endpoint() -> (World, EndpointRecord) {
    let w = support::engine::world().await;
    w.put_secret(SECRET_NAME, SECRET).await;
    let endpoint = w
        .events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            support::slug("billing"),
            settings(),
            &ctx(),
        )
        .await
        .expect("create the endpoint");
    (w, endpoint)
}

/// The headers a provider sends for `body` signed with `secret` at `timestamp`.
pub struct Signed {
    pub signature: String,
    pub timestamp: String,
    pub event_id: String,
    pub body: Vec<u8>,
}

impl Signed {
    /// A request for `event_type` signed with `secret`: the type is covered by the signature, so
    /// the request must name the same type (`request(Some(event_type))`) to verify.
    pub fn typed(
        secret: &str,
        timestamp: i64,
        event_id: &str,
        event_type: &str,
        body: &str,
    ) -> Self {
        Self {
            signature: support::signed::inbound(
                secret,
                timestamp,
                event_id,
                event_type,
                body.as_bytes(),
            ),
            timestamp: timestamp.to_string(),
            event_id: event_id.to_owned(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn request<'a>(&'a self, event_type: Option<&'a str>) -> WebhookRequest<'a> {
        WebhookRequest {
            trace: None,
            signature: Some(&self.signature),
            timestamp: Some(&self.timestamp),
            event_id: Some(&self.event_id),
            event_type,
            subject: None,
            correlation_key: None,
            body: &self.body,
        }
    }
}

pub fn now_seconds() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

pub fn event_type(text: &str) -> EventType {
    EventType::parse(text).expect("event type")
}

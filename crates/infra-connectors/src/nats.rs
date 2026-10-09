//! The NATS JetStream consumer (`FR-004-005`).
//!
//! A durable pull consumer with explicit acknowledgements: a message is acknowledged after the
//! event it became is in the inbox, and left to be delivered again (after a pause) otherwise. The
//! stream keeps the position, so the platform keeps none.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, AckKind, consumer::AckPolicy, consumer::pull};
use async_nats::{ConnectOptions, ServerAddr};
use futures_util::StreamExt as _;
use sagat_application::connectors::{ConnectorRuntime, RawMessage};
use sagat_domain::connectors::ConnectorSettings;
use tokio::sync::watch;

use crate::credentials::Credentials;
use crate::ingest::{Ingest, Outcome, Pacer};

/// Why a consumer stopped reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    /// It was told to.
    Asked,
    /// It failed, with a class of failure to show (never the broker's words).
    Failed(&'static str),
}

/// How long a message that could not be taken is left before the broker offers it again.
const RETRY: Duration = Duration::from_secs(5);
/// How long the server waits for an acknowledgement before it offers a message again.
const ACK_WAIT: Duration = Duration::from_secs(30);

/// Whether the credentials contain something that must not cross a connection nobody encrypts.
/// (A `.creds` file signs a challenge of the server and sends no secret.)
fn sends_a_secret(credentials: &Credentials) -> bool {
    credentials.text("creds").is_none()
        && (credentials.text("token").is_some()
            || (credentials.text("user").is_some() && credentials.text("password").is_some()))
}

fn options(credentials: &Credentials) -> Result<ConnectOptions, &'static str> {
    let options = ConnectOptions::new().name("sagat-octopus-orchestrator");
    if let Some(creds) = credentials.text("creds") {
        return options
            .credentials(creds)
            .map_err(|_| "the NATS credentials file is not valid");
    }
    if let Some(token) = credentials.text("token") {
        return Ok(options.token(token.to_owned()));
    }
    if let (Some(user), Some(password)) = (credentials.text("user"), credentials.text("password")) {
        return Ok(options.user_and_password(user.to_owned(), password.to_owned()));
    }
    Ok(options)
}

/// Reads the stream until told to stop or until something fails.
pub async fn run<R: ConnectorRuntime>(
    ingest: Arc<Ingest<R>>,
    credentials: Credentials,
    mut stop: watch::Receiver<bool>,
) -> Stopped {
    let connector = ingest.connector();
    let ConnectorSettings::Nats {
        servers,
        stream,
        consumer,
        filter_subject,
    } = &connector.settings
    else {
        return Stopped::Failed("the connector is not a NATS connector");
    };
    if sends_a_secret(&credentials) && !servers.iter().all(sagat_domain::connectors::Server::tls) {
        return Stopped::Failed(
            "credentials are not sent to a server without TLS (use a tls:// address)",
        );
    }
    let options = match options(&credentials) {
        Ok(options) => options,
        Err(why) => return Stopped::Failed(why),
    };
    let addresses: Vec<ServerAddr> = servers
        .iter()
        .filter_map(|s| {
            format!("{}://{}", if s.tls() { "tls" } else { "nats" }, s.address())
                .parse()
                .ok()
        })
        .collect();
    let Ok(client) = options.connect(addresses).await else {
        return Stopped::Failed("could not connect to the NATS servers");
    };
    let context = jetstream::new(client);
    let Ok(stream) = context.get_stream(stream.clone()).await else {
        return Stopped::Failed("the NATS stream does not exist or may not be read");
    };
    // What the server may have delivered and not heard back about is bounded by what this reader
    // can take in half of the time it has to answer, at its pace: messages prefetched faster than
    // they are taken would otherwise outlast the acknowledgement wait and be offered a second time.
    let in_flight = i64::from(connector.rate_per_second)
        .saturating_mul(ACK_WAIT.as_secs() as i64 / 2)
        .clamp(10, 1_000);
    let config = pull::Config {
        durable_name: Some(consumer.clone()),
        filter_subject: filter_subject.clone().unwrap_or_default(),
        ack_policy: AckPolicy::Explicit,
        ack_wait: ACK_WAIT,
        max_ack_pending: in_flight,
        ..pull::Config::default()
    };
    let Ok(consumer) = stream.get_or_create_consumer(consumer, config).await else {
        return Stopped::Failed("could not create the NATS consumer");
    };
    let Ok(mut messages) = consumer
        .stream()
        .max_messages_per_batch(usize::try_from(in_flight).unwrap_or(10))
        .messages()
        .await
    else {
        return Stopped::Failed("could not start reading the NATS consumer");
    };
    let mut pacer = Pacer::new(connector.rate_per_second);
    let stream_name = stream.cached_info().config.name.clone();
    loop {
        let next = tokio::select! {
            () = crate::supervisor::stopped(&mut stop) => {
                ingest.flush(true).await;
                return Stopped::Asked;
            }
            // Quiet: write down what has been read.
            () = tokio::time::sleep(Duration::from_secs(1)) => {
                ingest.flush(false).await;
                continue;
            }
            next = messages.next() => next,
        };
        let Some(Ok(message)) = next else {
            return Stopped::Failed("the NATS connection was lost");
        };
        pacer.wait().await;
        let sequence = message
            .info()
            .map(|i| i.stream_sequence)
            .unwrap_or_default();
        let position = format!("{stream_name}:{sequence}");
        let headers: BTreeMap<String, String> = message
            .headers
            .as_ref()
            .map(|h| {
                h.iter()
                    .filter_map(|(name, values)| {
                        values
                            .iter()
                            .next()
                            .map(|v| (name.to_string(), v.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The id the publisher gave the message, so a message published twice is one event.
        let fallback_id = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("nats-msg-id"))
            .map_or_else(|| position.clone(), |(_, v)| v.clone());
        let raw = RawMessage {
            position,
            fallback_id,
            headers,
            body: message.payload.to_vec(),
        };
        let outcome = ingest.ingest(&raw, None).await;
        let acknowledged = if outcome.done() {
            message.ack().await
        } else {
            message.ack_with(AckKind::Nak(Some(RETRY))).await
        };
        if acknowledged.is_err() {
            // The server did not hear: it offers the message again, and the inbox knows it.
            tracing::warn!(connector = %ingest.connector().id, "a NATS acknowledgement was not delivered");
        }
        if matches!(outcome, Outcome::Superseded) {
            return Stopped::Failed("another consumer reads this stream");
        }
    }
}

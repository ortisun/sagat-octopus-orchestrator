//! NATS JetStream adapter of the outbox's [`EventPublisher`].
//!
//! NATS carries hints and events, never the only copy of anything: PostgreSQL stays the authority
//! (ADR-0002), so a lost, duplicated or late message costs latency, not correctness. The adapter
//! therefore does three things only: keep the stream in place, publish with a stable
//! `Nats-Msg-Id` so the server drops the duplicates an at-least-once outbox produces, and map every
//! failure to "unavailable, retry" or "rejected".

use std::time::Duration;

use async_nats::jetstream::{self, stream};
use async_nats::{ConnectOptions, HeaderMap};
use sagat_application::maintenance::{EventPublisher, PublishError};
use tokio::sync::OnceCell;

/// Subjects of the stream: everything the platform publishes lives under this prefix.
pub const SUBJECT_FILTER: &str = "sagat.v1.>";

/// How to reach NATS and how the stream is kept.
#[derive(Clone)]
pub struct NatsConfig {
    /// `nats://host:port`. May carry credentials, so it is never logged or put in `Debug`.
    pub url: String,
    /// Name of the JetStream stream that holds the platform's subjects.
    pub stream: String,
    /// Path to a `.creds` file (JWT and NKey seed), when the server requires authentication.
    pub credentials_file: Option<String>,
    /// Refuse to connect without TLS.
    pub require_tls: bool,
    /// How long a message is kept for consumers that were away.
    pub max_age: Duration,
    /// Window in which the server drops a message id it has already seen.
    pub duplicate_window: Duration,
    /// Copies of the stream in a cluster (1 for a single node).
    pub replicas: usize,
    /// How long to wait for the server's acknowledgement of one message.
    pub ack_timeout: Duration,
}

impl NatsConfig {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            stream: "SAGAT".to_owned(),
            credentials_file: None,
            require_tls: false,
            max_age: Duration::from_secs(3600),
            duplicate_window: Duration::from_secs(120),
            replicas: 1,
            ack_timeout: Duration::from_secs(5),
        }
    }
}

impl std::fmt::Debug for NatsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsConfig")
            .field("url", &"<redacted>")
            .field("stream", &self.stream)
            .field("credentials_file", &self.credentials_file.is_some())
            .field("require_tls", &self.require_tls)
            .finish_non_exhaustive()
    }
}

/// Why the adapter could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum NatsError {
    /// The URL or credentials are unusable; never contains them.
    #[error("invalid NATS configuration: {0}")]
    Config(&'static str),
    #[error("could not connect to NATS")]
    Connect,
}

/// A JetStream publisher. Cloning is cheap and shares the connection.
#[derive(Clone)]
pub struct NatsPublisher {
    jetstream: jetstream::Context,
    config: NatsConfig,
    stream_ready: std::sync::Arc<OnceCell<()>>,
}

impl std::fmt::Debug for NatsPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsPublisher")
            .field("stream", &self.config.stream)
            .finish_non_exhaustive()
    }
}

impl NatsPublisher {
    /// Connects without waiting: the client keeps retrying in the background, so the engine starts
    /// (and keeps accumulating its outbox) while NATS is down. The stream is created on first use.
    ///
    /// # Errors
    /// [`NatsError`] for an unusable configuration.
    pub async fn connect(config: NatsConfig) -> Result<Self, NatsError> {
        if config.url.trim().is_empty() {
            return Err(NatsError::Config("the URL is empty"));
        }
        if config.stream.is_empty()
            || !config
                .stream
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(NatsError::Config("the stream name is invalid"));
        }
        let mut options = ConnectOptions::new()
            .retry_on_initial_connect()
            .connection_timeout(Duration::from_secs(5))
            .name("sagat-execution-engine")
            .require_tls(config.require_tls)
            .event_callback(|event| async move {
                // Events carry no addresses or credentials.
                tracing::debug!(event = %event, "NATS connection event");
            });
        if let Some(path) = &config.credentials_file {
            options = options
                .credentials_file(path)
                .await
                .map_err(|_| NatsError::Config("the credentials file cannot be read"))?;
        }
        let client = options
            .connect(config.url.as_str())
            .await
            .map_err(|_| NatsError::Connect)?;
        Ok(Self {
            jetstream: jetstream::ContextBuilder::new()
                .timeout(config.ack_timeout)
                .build(client),
            config,
            stream_ready: std::sync::Arc::new(OnceCell::new()),
        })
    }

    /// Creates the stream if it is missing (idempotent). Called before the first publish.
    async fn ensure_stream(&self) -> Result<(), PublishError> {
        self.stream_ready
            .get_or_try_init(|| async {
                self.jetstream
                    .get_or_create_stream(stream::Config {
                        name: self.config.stream.clone(),
                        subjects: vec![SUBJECT_FILTER.to_owned()],
                        retention: stream::RetentionPolicy::Limits,
                        storage: stream::StorageType::File,
                        max_age: self.config.max_age,
                        duplicate_window: self.config.duplicate_window,
                        num_replicas: self.config.replicas,
                        ..Default::default()
                    })
                    .await
                    .map(|_| ())
                    .map_err(|error| {
                        tracing::warn!(%error, "could not ensure the JetStream stream");
                        PublishError::Unavailable
                    })
            })
            .await
            .copied()
    }

    /// The stream, for consumers (tests, tooling).
    ///
    /// # Errors
    /// [`PublishError::Unavailable`] when NATS cannot be reached.
    pub async fn stream(&self) -> Result<jetstream::stream::Stream, PublishError> {
        self.ensure_stream().await?;
        self.jetstream
            .get_stream(&self.config.stream)
            .await
            .map_err(|_| PublishError::Unavailable)
    }
}

impl EventPublisher for NatsPublisher {
    async fn publish(
        &self,
        subject: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<(), PublishError> {
        if !subject.starts_with("sagat.v1.") {
            return Err(PublishError::Rejected(
                "the subject is outside the platform's",
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", message_id);
        // One bound for everything the call may wait on (creating the stream included), so an
        // unreachable broker costs one acknowledgement timeout, not several.
        let attempt = async {
            self.ensure_stream().await?;
            let ack = self
                .jetstream
                .publish_with_headers(subject.to_owned(), headers, payload.to_vec().into())
                .await
                .map_err(|error| {
                    tracing::debug!(%error, "JetStream publish failed");
                    PublishError::Unavailable
                })?;
            ack.await.map_err(|error| {
                tracing::debug!(%error, "JetStream did not acknowledge");
                PublishError::Unavailable
            })
        };
        match tokio::time::timeout(self.config.ack_timeout, attempt).await {
            Ok(Ok(_ack)) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                tracing::debug!("JetStream publish timed out");
                Err(PublishError::Unavailable)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_never_prints_the_url_or_the_credentials_path() {
        let mut config = NatsConfig::new("nats://user:s3cret@nats.internal:4222");
        config.credentials_file = Some("/etc/nats/secret.creds".to_owned());
        let shown = format!("{config:?}");
        assert!(
            !shown.contains("s3cret") && !shown.contains("nats.internal"),
            "{shown}"
        );
        assert!(!shown.contains("secret.creds"), "{shown}");
    }

    #[tokio::test]
    async fn an_unusable_configuration_is_refused_before_connecting() {
        assert!(matches!(
            NatsPublisher::connect(NatsConfig::new("  ")).await,
            Err(NatsError::Config(_))
        ));
        let mut bad_stream = NatsConfig::new("nats://127.0.0.1:1");
        bad_stream.stream = "has space".to_owned();
        assert!(matches!(
            NatsPublisher::connect(bad_stream).await,
            Err(NatsError::Config(_))
        ));
        let mut no_file = NatsConfig::new("nats://127.0.0.1:1");
        no_file.credentials_file = Some("/definitely/not/here.creds".to_owned());
        assert!(matches!(
            NatsPublisher::connect(no_file).await,
            Err(NatsError::Config(_))
        ));
    }
}

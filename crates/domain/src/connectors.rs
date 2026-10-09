//! Connectors: where events come from when they are not posted to the platform (`FR-004-005`).
//!
//! A connector consumes a topic or a queue of a broker the tenant runs and admits what it reads to
//! the inbox, under the connector's slug as the source. Acknowledging a message to the broker comes
//! after the event is in the inbox and never before, so a crash can repeat a message but not lose
//! one, and the inbox's deduplication makes the repeat harmless.

use std::fmt;

use crate::identity::Slug;

uuid_id!(
    /// Identifier of a connector.
    ConnectorId
);

/// Longest broker address list, topic, queue or subject accepted.
const MAX_TEXT: usize = 512;
/// Most brokers one connector names.
pub const MAX_SERVERS: usize = 8;
/// Largest rate a connector may be told to ingest at, per second.
pub const MAX_INGEST_RATE: u32 = 10_000;
/// What a connector ingests per second unless it is told otherwise.
pub const DEFAULT_INGEST_RATE: u32 = 200;

/// Why a connector setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConnectorError {
    #[error("a broker address is host:port, with an optional nats://, tls:// or kafka:// prefix")]
    InvalidServer,
    #[error("a connector names between 1 and {MAX_SERVERS} brokers")]
    InvalidServers,
    #[error(
        "a name (stream, consumer, topic, subject) is printable ASCII without spaces, at most {MAX_TEXT} characters"
    )]
    InvalidName,
    #[error("a queue URL is an https URL of a queue")]
    InvalidQueueUrl,
    #[error("a region is a short lower-case name such as us-east-1")]
    InvalidRegion,
    #[error("the ingest rate is between 1 and {MAX_INGEST_RATE} messages per second")]
    InvalidRate,
    #[error("the settings do not fit the kind of connector")]
    WrongKind,
}

/// What kind of broker a connector reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectorKind {
    /// A NATS JetStream stream, read through a durable pull consumer.
    Nats,
    /// A Kafka topic, read with the offsets kept in the platform's database.
    Kafka,
    /// An Amazon SQS (or compatible) queue.
    Sqs,
}

impl ConnectorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nats => "NATS",
            Self::Kafka => "KAFKA",
            Self::Sqs => "SQS",
        }
    }
}

impl fmt::Display for ConnectorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ConnectorKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "NATS" => Ok(Self::Nats),
            "KAFKA" => Ok(Self::Kafka),
            "SQS" => Ok(Self::Sqs),
            _ => Err(()),
        }
    }
}

/// Whether a connector is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorStatus {
    Active,
    /// Not read: nothing is consumed, and nothing the broker holds is lost.
    Paused,
}

impl ConnectorStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
        }
    }
}

impl std::str::FromStr for ConnectorStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "PAUSED" => Ok(Self::Paused),
            _ => Err(()),
        }
    }
}

/// A broker address, `host:port`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Server {
    host: String,
    port: u16,
    /// Whether the connection is encrypted (`tls://`, or a prefix that says so).
    tls: bool,
}

impl Server {
    /// Accepts `host:port`, with `nats://`, `tls://`, `kafka://` or `ssl://` in front; `tls://` and
    /// `ssl://` mean an encrypted connection.
    ///
    /// # Errors
    /// [`ConnectorError::InvalidServer`].
    pub fn parse(text: &str) -> Result<Self, ConnectorError> {
        let (tls, rest) = match text.split_once("://") {
            Some(("tls" | "ssl", rest)) => (true, rest),
            Some(("nats" | "kafka", rest)) => (false, rest),
            Some(_) => return Err(ConnectorError::InvalidServer),
            None => (false, text),
        };
        let (host, port) = rest.rsplit_once(':').ok_or(ConnectorError::InvalidServer)?;
        let port: u16 = port.parse().map_err(|_| ConnectorError::InvalidServer)?;
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if port == 0
            || bare.is_empty()
            || bare.len() > 253
            || !bare
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return Err(ConnectorError::InvalidServer);
        }
        Ok(Self {
            host: bare.to_ascii_lowercase(),
            port,
            tls,
        })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub const fn tls(&self) -> bool {
        self.tls
    }

    /// What the egress policy judges: an encrypted connection counts as `https`, a plaintext one as
    /// `http`, so a plaintext broker needs the policy to allow plain HTTP, which is a decision an
    /// administrator makes on purpose.
    #[must_use]
    pub fn destination(&self) -> crate::egress::Destination<'_> {
        crate::egress::Destination {
            scheme: if self.tls { "https" } else { "http" },
            host: &self.host,
            port: self.port,
        }
    }

    /// The address as it is written back.
    #[must_use]
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn name(text: &str) -> Result<String, ConnectorError> {
    if text.is_empty() || text.len() > MAX_TEXT || !text.chars().all(|c| c.is_ascii_graphic()) {
        Err(ConnectorError::InvalidName)
    } else {
        Ok(text.to_owned())
    }
}

/// Where a Kafka consumer starts when the platform has no offset for a partition yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFrom {
    Earliest,
    Latest,
}

/// The settings that are specific to a kind. Credentials are never here: they are a secret of the
/// namespace, named by the connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorSettings {
    Nats {
        servers: Vec<Server>,
        stream: String,
        /// The durable consumer; created if the stream does not have it.
        consumer: String,
        /// Narrows the stream to a subject filter, such as `orders.>`.
        filter_subject: Option<String>,
    },
    Kafka {
        servers: Vec<Server>,
        topic: String,
        start_from: StartFrom,
    },
    Sqs {
        queue_url: String,
        region: String,
    },
}

impl ConnectorSettings {
    /// # Errors
    /// [`ConnectorError`] for the first setting that is not acceptable.
    pub fn nats(
        servers: &[String],
        stream: &str,
        consumer: &str,
        filter_subject: Option<&str>,
    ) -> Result<Self, ConnectorError> {
        Ok(Self::Nats {
            servers: parse_servers(servers)?,
            stream: name(stream)?,
            consumer: name(consumer)?,
            filter_subject: filter_subject.map(name).transpose()?,
        })
    }

    /// # Errors
    /// [`ConnectorError`].
    pub fn kafka(
        servers: &[String],
        topic: &str,
        start_from: StartFrom,
    ) -> Result<Self, ConnectorError> {
        Ok(Self::Kafka {
            servers: parse_servers(servers)?,
            topic: name(topic)?,
            start_from,
        })
    }

    /// # Errors
    /// [`ConnectorError`].
    pub fn sqs(queue_url: &str, region: &str) -> Result<Self, ConnectorError> {
        let url = crate::outbound::TargetUrl::parse(queue_url)
            .map_err(|_| ConnectorError::InvalidQueueUrl)?;
        let destination = url.destination();
        if destination.host.is_empty() {
            return Err(ConnectorError::InvalidQueueUrl);
        }
        if region.is_empty()
            || region.len() > 32
            || !region
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(ConnectorError::InvalidRegion);
        }
        Ok(Self::Sqs {
            queue_url: queue_url.to_owned(),
            region: region.to_owned(),
        })
    }

    #[must_use]
    pub const fn kind(&self) -> ConnectorKind {
        match self {
            Self::Nats { .. } => ConnectorKind::Nats,
            Self::Kafka { .. } => ConnectorKind::Kafka,
            Self::Sqs { .. } => ConnectorKind::Sqs,
        }
    }

    /// The addresses the connector connects to, for the egress policy.
    #[must_use]
    pub fn servers(&self) -> Vec<Server> {
        match self {
            Self::Nats { servers, .. } | Self::Kafka { servers, .. } => servers.clone(),
            Self::Sqs { queue_url, .. } => crate::outbound::TargetUrl::parse(queue_url)
                .map(|url| {
                    let d = url.destination();
                    vec![Server {
                        host: d.host.to_owned(),
                        port: d.port,
                        tls: d.scheme == "https",
                    }]
                })
                .unwrap_or_default(),
        }
    }
}

fn parse_servers(servers: &[String]) -> Result<Vec<Server>, ConnectorError> {
    if servers.is_empty() || servers.len() > MAX_SERVERS {
        return Err(ConnectorError::InvalidServers);
    }
    servers.iter().map(|s| Server::parse(s)).collect()
}

/// The slug of a connector is the source of the events it admits; these are not available.
pub const RESERVED_SLUGS: [&str; 2] = ["api", "cron"];

/// Whether `slug` can name a connector.
#[must_use]
pub fn is_reserved(slug: &Slug) -> bool {
    RESERVED_SLUGS.contains(&slug.as_str())
}

/// Checks a rate.
///
/// # Errors
/// [`ConnectorError::InvalidRate`].
pub fn check_rate(rate: u32) -> Result<u32, ConnectorError> {
    if (1..=MAX_INGEST_RATE).contains(&rate) {
        Ok(rate)
    } else {
        Err(ConnectorError::InvalidRate)
    }
}

/// Why a message was set aside instead of admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadLetter {
    /// The message names no event type and the connector has no default.
    NoType,
    /// The event type, the id, the subject or the correlation key is not acceptable.
    BadEnvelope,
    /// The body is not a JSON object.
    NotAnObject,
    /// The body is larger than an event may be.
    TooLarge,
    /// The id was used before for a different message.
    IdReused,
}

impl DeadLetter {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoType => "NO_EVENT_TYPE",
            Self::BadEnvelope => "BAD_ENVELOPE",
            Self::NotAnObject => "NOT_A_JSON_OBJECT",
            Self::TooLarge => "TOO_LARGE",
            Self::IdReused => "EVENT_ID_REUSED",
        }
    }
}

impl std::str::FromStr for DeadLetter {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        [
            Self::NoType,
            Self::BadEnvelope,
            Self::NotAnObject,
            Self::TooLarge,
            Self::IdReused,
        ]
        .into_iter()
        .find(|d| d.as_str() == value)
        .ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_a_host_and_a_port_and_says_whether_it_is_encrypted() {
        let plain = Server::parse("Broker-1.Example.com:9092").expect("server");
        assert_eq!(
            (plain.host(), plain.port(), plain.tls()),
            ("broker-1.example.com", 9092, false)
        );
        assert_eq!(
            plain.destination().scheme,
            "http",
            "a plaintext broker counts as plain HTTP for the policy"
        );
        let secure = Server::parse("tls://nats.example.com:4222").expect("server");
        assert!(secure.tls());
        assert_eq!(secure.destination().scheme, "https");
        assert!(
            !Server::parse("nats://nats.example.com:4222")
                .expect("server")
                .tls()
        );
        assert_eq!(
            Server::parse("[2001:db8::1]:9092").expect("server").host(),
            "2001:db8::1"
        );
        for bad in [
            "",
            "nohost",
            ":9092",
            "host:0",
            "host:70000",
            "http://host:80",
            "user@host:9092",
            "host name:9092",
            "ho\u{e4}st:9092",
        ] {
            assert_eq!(
                Server::parse(bad),
                Err(ConnectorError::InvalidServer),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_settings_of_each_kind_are_checked() {
        let servers = vec!["kafka-1.example.com:9092".to_owned()];
        assert!(ConnectorSettings::kafka(&servers, "orders", StartFrom::Earliest).is_ok());
        assert_eq!(
            ConnectorSettings::kafka(&[], "orders", StartFrom::Earliest),
            Err(ConnectorError::InvalidServers)
        );
        assert_eq!(
            ConnectorSettings::kafka(&servers, "has space", StartFrom::Latest),
            Err(ConnectorError::InvalidName)
        );
        assert_eq!(
            ConnectorSettings::kafka(
                &vec!["a.example.com:1".to_owned(); 9],
                "t",
                StartFrom::Latest
            ),
            Err(ConnectorError::InvalidServers)
        );
        assert!(ConnectorSettings::nats(&servers, "orders", "sagat", Some("orders.>")).is_ok());
        assert_eq!(
            ConnectorSettings::nats(&servers, "", "sagat", None),
            Err(ConnectorError::InvalidName)
        );
        assert!(
            ConnectorSettings::sqs(
                "https://sqs.us-east-1.amazonaws.com/123/orders",
                "us-east-1"
            )
            .is_ok()
        );
        assert_eq!(
            ConnectorSettings::sqs("sqs.us-east-1.amazonaws.com/123/orders", "us-east-1"),
            Err(ConnectorError::InvalidQueueUrl)
        );
        assert_eq!(
            ConnectorSettings::sqs("https://sqs.example.com/q", "US East"),
            Err(ConnectorError::InvalidRegion)
        );
        let sqs = ConnectorSettings::sqs(
            "https://sqs.us-east-1.amazonaws.com/123/orders",
            "us-east-1",
        )
        .expect("sqs");
        assert_eq!(sqs.kind(), ConnectorKind::Sqs);
        assert_eq!(
            sqs.servers()[0].port(),
            443,
            "the queue is a destination like any other"
        );
    }

    #[test]
    fn rates_and_reserved_names_are_bounded() {
        assert_eq!(check_rate(1), Ok(1));
        assert_eq!(check_rate(MAX_INGEST_RATE), Ok(MAX_INGEST_RATE));
        assert_eq!(check_rate(0), Err(ConnectorError::InvalidRate));
        assert_eq!(
            check_rate(MAX_INGEST_RATE + 1),
            Err(ConnectorError::InvalidRate)
        );
        assert!(is_reserved(&Slug::parse("api").expect("slug")));
        assert!(is_reserved(&Slug::parse("cron").expect("slug")));
        assert!(!is_reserved(&Slug::parse("orders").expect("slug")));
        for letter in [
            DeadLetter::NoType,
            DeadLetter::BadEnvelope,
            DeadLetter::NotAnObject,
            DeadLetter::TooLarge,
            DeadLetter::IdReused,
        ] {
            assert_eq!(letter.as_str().parse(), Ok(letter));
        }
    }
}

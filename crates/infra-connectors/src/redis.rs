//! Redis Streams protocol shared by internal hints and guarded callback publishing.
//! Every attempt owns its connection; database/authentication never change on a shared connection.
use redis::{AsyncConnectionConfig, IntoConnectionInfo as _};
use sagat_application::maintenance::{EventPublisher, PublishError};
use std::time::Duration;

pub const MAX_PAYLOAD: usize = 256 * 1024;
pub const MAX_STREAM_LEN: u64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RedisError {
    #[error("invalid Redis settings")]
    Invalid,
    #[error("Redis connection or acknowledgement failed")]
    Unavailable,
    #[error("Redis publish deadline exceeded")]
    Timeout,
    #[error("Redis refused the append")]
    Rejected,
}

#[derive(Clone)]
pub struct RedisConfig {
    url: String,
    pub stream: String,
    pub max_len: u64,
    pub timeout: Duration,
}
impl std::fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConfig")
            .field("endpoint", &"<redacted>")
            .field("stream", &self.stream)
            .field("max_len", &self.max_len)
            .field("timeout", &self.timeout)
            .finish()
    }
}
impl RedisConfig {
    pub fn new(
        url: &str,
        stream: &str,
        max_len: u64,
        timeout: Duration,
        require_tls: bool,
    ) -> Result<Self, RedisError> {
        let parsed = url::Url::parse(url).map_err(|_| RedisError::Invalid)?;
        if url.len() > 16 * 1024
            || parsed.port() == Some(0)
            || !matches!(parsed.scheme(), "redis" | "rediss")
            || parsed.host_str().is_none()
            || parsed.fragment().is_some()
            || parsed.query().is_some()
            || (require_tls && parsed.scheme() != "rediss")
            || !valid_stream(stream)
            || !(1..=MAX_STREAM_LEN).contains(&max_len)
            || timeout < Duration::from_secs(1)
            || timeout > Duration::from_secs(30)
        {
            return Err(RedisError::Invalid);
        }
        let connection = url
            .into_connection_info()
            .map_err(|_| RedisError::Invalid)?;
        if !(0..=15).contains(&connection.redis_settings().db()) {
            return Err(RedisError::Invalid);
        }
        Ok(Self {
            url: url.to_owned(),
            stream: stream.to_owned(),
            max_len,
            timeout,
        })
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

pub use sagat_domain::outbound::valid_redis_stream as valid_stream;

/// One bounded connection lifecycle per attempt: a timed-out command may have reached Redis,
/// but no reusable client or unbounded set of outstanding commands is retained by this adapter.
pub async fn append(
    info: redis::ConnectionInfo,
    stream: &str,
    max_len: Option<u64>,
    fields: &[(&str, &[u8])],
    timeout: Duration,
    ca: Option<Vec<u8>>,
    addresses: Option<Vec<std::net::SocketAddr>>,
) -> Result<String, RedisError> {
    if !(0..=15).contains(&info.redis_settings().db())
        || !valid_stream(stream)
        || fields.is_empty()
        || fields.len() > 32
        || fields
            .iter()
            .any(|(key, _)| key.is_empty() || key.len() > 64)
        || fields.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() > MAX_PAYLOAD + 16 * 1024
        || max_len.is_some_and(|n| !(1..=MAX_STREAM_LEN).contains(&n))
        || timeout.is_zero()
        || timeout > Duration::from_secs(30)
    {
        return Err(RedisError::Invalid);
    }
    let operation = async {
        let settings = info
            .redis_settings()
            .clone()
            .set_skip_set_lib_name()
            .set_protocol(redis::ProtocolVersion::RESP2);
        let info = info.set_redis_settings(settings);
        let client = match ca {
            Some(root_cert) => redis::Client::build_with_tls(
                info,
                redis::TlsCertificates {
                    client_tls: None,
                    root_cert: Some(root_cert),
                },
            ),
            None => redis::Client::open(info),
        }
        .map_err(|_| RedisError::Invalid)?;
        let options = AsyncConnectionConfig::new()
            .set_connection_timeout(Some(timeout))
            .set_response_timeout(Some(timeout));
        // Redis-rs races every resolver result. Supply one candidate so a DNS answer cannot
        // amplify sockets. All checked addresses retain the original hostname for TLS.
        let options = options.set_dns_resolver(PinnedAddresses(addresses));
        let mut connection = client
            .get_multiplexed_async_connection_with_config(&options)
            .await
            .map_err(classify)?;
        let mut command = redis::cmd("XADD");
        command.arg(stream);
        if let Some(length) = max_len {
            command.arg("MAXLEN").arg("=").arg(length);
        }
        command.arg("*");
        for (key, value) in fields {
            command.arg(key).arg(value);
        }
        let id: String = command
            .query_async(&mut connection)
            .await
            .map_err(classify)?;
        let valid = id.split_once('-').is_some_and(|(ms, seq)| {
            !ms.is_empty()
                && !seq.is_empty()
                && ms.bytes().all(|b| b.is_ascii_digit())
                && seq.bytes().all(|b| b.is_ascii_digit())
                && (ms.len() == 1 || !ms.starts_with('0'))
                && (seq.len() == 1 || !seq.starts_with('0'))
                && ms.parse::<u64>().is_ok()
                && seq.parse::<u64>().is_ok()
                && id != "0-0"
        });
        if !valid {
            return Err(RedisError::Rejected);
        }
        Ok(id)
    };
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| RedisError::Timeout)?
}

#[derive(Debug, Clone)]
pub struct RedisPublisher {
    config: RedisConfig,
}
impl RedisPublisher {
    pub fn new(config: RedisConfig) -> Self {
        Self { config }
    }
}
impl EventPublisher for RedisPublisher {
    async fn publish(
        &self,
        subject: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<(), PublishError> {
        if !subject.starts_with("sagat.v1.")
            || subject.len() > 2048
            || message_id.is_empty()
            || message_id.len() > 256
            || payload.len() > MAX_PAYLOAD
        {
            return Err(PublishError::Rejected(
                "invalid internal Redis notification",
            ));
        }
        let info = self
            .config
            .url()
            .into_connection_info()
            .map_err(|_| PublishError::Rejected("invalid Redis configuration"))?;
        append(
            info,
            &self.config.stream,
            Some(self.config.max_len),
            &[
                ("message-id", message_id.as_bytes()),
                ("subject", subject.as_bytes()),
                ("payload", payload),
            ],
            self.config.timeout,
            None,
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            RedisError::Timeout | RedisError::Unavailable => PublishError::Unavailable,
            RedisError::Invalid | RedisError::Rejected => {
                PublishError::Rejected("Redis refused the internal notification")
            }
        })
    }
}

fn classify(error: redis::RedisError) -> RedisError {
    if error.is_timeout() {
        RedisError::Timeout
    } else if error.is_io_error() {
        RedisError::Unavailable
    } else {
        RedisError::Rejected
    }
}

/// No second DNS lookup is allowed after namespace policy checks.
struct PinnedAddresses(Option<Vec<std::net::SocketAddr>>);
impl redis::io::AsyncDNSResolver for PinnedAddresses {
    fn resolve<'a, 'b: 'a>(
        &'a self,
        host: &'b str,
        port: u16,
    ) -> redis::RedisFuture<'a, Box<dyn Iterator<Item = std::net::SocketAddr> + Send + 'a>> {
        Box::pin(async move {
            let addresses: Box<dyn Iterator<Item = std::net::SocketAddr> + Send + 'a> =
                match &self.0 {
                    Some(addresses) => Box::new(
                        addresses
                            .iter()
                            .take(1)
                            .copied()
                            .filter(move |address| address.port() == port),
                    ),
                    None => Box::new(tokio::net::lookup_host((host, port)).await?.take(1)),
                };
            Ok(addresses)
        })
    }
}

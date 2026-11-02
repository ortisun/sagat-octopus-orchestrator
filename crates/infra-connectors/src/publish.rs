//! Guarded outbound broker publishers (`FR-004-005`).
//!
//! A successful call means the destination acknowledged the message: a JetStream PubAck, a Kafka
//! produce response, a successful SQS `SendMessage` response, or a Redis XADD stream id. The caller keeps the durable
//! delivery id stable across retries; each protocol carries it as its deduplication metadata.

use std::collections::BTreeMap;

use async_nats::{HeaderMap, ServerAddr, jetstream};
use rskafka::client::ClientBuilder;
use rskafka::client::partition::{Compression, UnknownTopicHandling};
use rskafka::record::Record;
use sagat_application::egress::EgressStore;
use sagat_application::outbound::{BrokerFailure, BrokerPublisher, BrokerRequest, SendFailure};
use sagat_domain::connectors::Server;
use sagat_domain::outbound::{BrokerDestination, TargetUrl};

use crate::credentials::Credentials;
use crate::guard::{BrokerGuard, PolicyGuard};
use crate::webhook::{ClientFactory, Clients, Guarded};

const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Publishes through broker-native clients after applying the namespace's current egress policy.
pub struct Publisher<G, S, F = Guarded> {
    guard: G,
    clients: Clients<S, F>,
}

impl<G, S, F> std::fmt::Debug for Publisher<G, S, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Publisher").finish_non_exhaustive()
    }
}

impl<S> Publisher<PolicyGuard<S>, S, Guarded>
where
    S: EgressStore + Clone,
{
    #[must_use]
    pub fn new(store: S) -> Self {
        Self {
            guard: PolicyGuard::new(store.clone()),
            clients: Clients::new(store),
        }
    }
}

impl<G, S, F> Publisher<G, S, F> {
    #[must_use]
    pub const fn with_parts(guard: G, clients: Clients<S, F>) -> Self {
        Self { guard, clients }
    }
}

impl<G, S, F> BrokerPublisher for Publisher<G, S, F>
where
    G: BrokerGuard,
    S: EgressStore + Send + Sync,
    F: ClientFactory,
{
    async fn publish(&self, request: &BrokerRequest) -> Result<(), BrokerFailure> {
        let operation = async {
            if request.body.len() > MAX_PAYLOAD_BYTES
                || request.headers.len() > MAX_HEADERS
                || request
                    .headers
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>()
                    > MAX_HEADER_BYTES
            {
                return Err(BrokerFailure::Rejected);
            }
            let credentials = match &request.credentials {
                Some(value) => {
                    Credentials::parse(value.expose()).map_err(|_| BrokerFailure::Rejected)?
                }
                None => Credentials::none(),
            };
            if !matches!(&request.destination, BrokerDestination::Redis { .. })
                && !self
                    .guard
                    .permits(
                        request.tenant_id,
                        request.namespace_id,
                        &request.destination.servers(),
                    )
                    .await
            {
                return Err(BrokerFailure::Denied);
            }
            match &request.destination {
                BrokerDestination::Redis {
                    server,
                    stream,
                    database,
                } => {
                    self.redis(request, server, stream, *database, &credentials)
                        .await
                }
                BrokerDestination::Nats { servers, subject } => {
                    self.nats(request, servers, subject, &credentials).await
                }
                BrokerDestination::Kafka { servers, topic } => {
                    self.kafka(request, servers, topic, &credentials).await
                }
                BrokerDestination::Sqs { queue_url, region } => {
                    self.sqs(request, queue_url, region, &credentials).await
                }
            }
        };
        tokio::time::timeout(request.timeout, operation)
            .await
            .map_err(|_| BrokerFailure::Timeout)?
    }
}

impl<G, S, F> Publisher<G, S, F>
where
    G: BrokerGuard,
    S: EgressStore + Send + Sync,
    F: ClientFactory,
{
    async fn redis(
        &self,
        request: &BrokerRequest,
        server: &Server,
        stream: &str,
        database: u8,
        credentials: &Credentials,
    ) -> Result<(), BrokerFailure> {
        use redis::IntoConnectionInfo as _;
        let credentials = credentials
            .redis(request.credentials.is_some())
            .map_err(|()| BrokerFailure::Rejected)?;
        if (credentials.password.is_some() || credentials.ca_cert.is_some()) && !server.tls() {
            return Err(BrokerFailure::Rejected);
        }
        // The worker's metadata is a closed wire contract. Never permit duplicate fields or a
        // header named body to shadow the event envelope in a Redis map.
        let mut names = std::collections::BTreeSet::new();
        for (name, value) in &request.headers {
            if !matches!(
                name.as_str(),
                "content-type"
                    | "event-id"
                    | "event-type"
                    | "delivery-id"
                    | "attempt"
                    | "schema-version"
                    | "traceparent"
            ) || value.is_empty()
                || !names.insert(name.as_str())
            {
                return Err(BrokerFailure::Rejected);
            }
        }
        if header(request, "content-type") != Some("application/json") {
            return Err(BrokerFailure::Rejected);
        }
        if [
            "content-type",
            "event-id",
            "event-type",
            "delivery-id",
            "attempt",
            "schema-version",
        ]
        .iter()
        .any(|name| !names.contains(name))
        {
            return Err(BrokerFailure::Rejected);
        }
        let addresses = self
            .guard
            .resolve(request.tenant_id, request.namespace_id, server)
            .await
            .map_err(|error| match error {
                crate::guard::ResolveFailure::Denied => BrokerFailure::Denied,
                crate::guard::ResolveFailure::Network => BrokerFailure::Network,
            })?;
        let info = sagat_domain::outbound::redis_endpoint(server)
            .into_connection_info()
            .map_err(|_| BrokerFailure::Rejected)?;
        let mut settings = info.redis_settings().clone().set_db(i64::from(database));
        if let Some(username) = credentials.username {
            settings = settings.set_username(username);
        }
        if let Some(password) = credentials.password {
            settings = settings.set_password(password);
        }
        let mut fields: Vec<_> = request
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_bytes()))
            .collect();
        fields.push(("body", &request.body));
        crate::redis::append(
            info.set_redis_settings(settings),
            stream,
            None,
            &fields,
            request.timeout,
            credentials.ca_cert.map(|s| s.as_bytes().to_vec()),
            Some(addresses),
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            crate::redis::RedisError::Timeout => BrokerFailure::Timeout,
            crate::redis::RedisError::Unavailable => BrokerFailure::Network,
            crate::redis::RedisError::Invalid | crate::redis::RedisError::Rejected => {
                BrokerFailure::Rejected
            }
        })
    }

    async fn nats(
        &self,
        request: &BrokerRequest,
        servers: &[Server],
        subject: &str,
        credentials: &Credentials,
    ) -> Result<(), BrokerFailure> {
        if crate::nats::sends_a_secret(credentials) && !servers.iter().all(Server::tls) {
            return Err(BrokerFailure::Rejected);
        }
        let options = crate::nats::options(credentials).map_err(|_| BrokerFailure::Rejected)?;
        let addresses: Vec<ServerAddr> = servers
            .iter()
            .map(|server| server.configured_address().parse())
            .collect::<Result<_, _>>()
            .map_err(|_| BrokerFailure::Rejected)?;
        let mut headers = HeaderMap::new();
        for (name, value) in &request.headers {
            headers.insert(name.as_str(), value.as_str());
        }
        let delivery_id = header(request, "delivery-id").ok_or(BrokerFailure::Rejected)?;
        headers.insert("Nats-Msg-Id", delivery_id);
        let operation = async {
            let client = options
                .connect(addresses)
                .await
                .map_err(|_| BrokerFailure::Network)?;
            // Keep the client-owning context alive until the server's PubAck arrives.
            let context = jetstream::new(client);
            let ack = context
                .publish_with_headers(subject.to_owned(), headers, request.body.clone().into())
                .await
                .map_err(|_| BrokerFailure::Network)?;
            ack.await.map_err(|error| {
                tracing::warn!(error.kind = %error.kind(), "NATS JetStream rejected an outbound publish");
                BrokerFailure::Rejected
            })?;
            Ok(())
        };
        tokio::time::timeout(request.timeout, operation)
            .await
            .map_err(|_| BrokerFailure::Timeout)?
    }

    async fn kafka(
        &self,
        request: &BrokerRequest,
        servers: &[Server],
        topic: &str,
        credentials: &Credentials,
    ) -> Result<(), BrokerFailure> {
        let mut builder = ClientBuilder::new(servers.iter().map(Server::address).collect());
        if let Some(sasl) = crate::kafka::sasl(credentials).map_err(|_| BrokerFailure::Rejected)? {
            if !servers.iter().all(Server::tls) {
                return Err(BrokerFailure::Rejected);
            }
            builder = builder.sasl_config(sasl);
        }
        if servers.iter().any(Server::tls) {
            builder = builder.tls_config(crate::kafka::tls());
        }
        let delivery_id = header(request, "delivery-id").ok_or(BrokerFailure::Rejected)?;
        let operation = async {
            let client = builder.build().await.map_err(|_| BrokerFailure::Network)?;
            let topic_info = client
                .list_topics()
                .await
                .map_err(|_| BrokerFailure::Network)?
                .into_iter()
                .find(|candidate| candidate.name == topic)
                .ok_or(BrokerFailure::Rejected)?;
            let partition_count = topic_info.partitions.len();
            if partition_count == 0 || partition_count > 256 {
                return Err(BrokerFailure::Rejected);
            }
            let index = usize::try_from(
                fnv1a(delivery_id.as_bytes()) % u64::try_from(partition_count).unwrap_or(1),
            )
            .unwrap_or(0);
            let partition = topic_info
                .partitions
                .iter()
                .nth(index)
                .copied()
                .ok_or(BrokerFailure::Rejected)?;
            let client = client
                .partition_client(topic.to_owned(), partition, UnknownTopicHandling::Retry)
                .await
                .map_err(|_| BrokerFailure::Network)?;
            let record = Record {
                key: Some(delivery_id.as_bytes().to_vec()),
                value: Some(request.body.clone()),
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_bytes().to_vec()))
                    .collect::<BTreeMap<_, _>>(),
                timestamp: chrono::Utc::now(),
            };
            client
                .produce(vec![record], Compression::NoCompression)
                .await
                .map_err(|_| BrokerFailure::Rejected)?;
            Ok(())
        };
        tokio::time::timeout(request.timeout, operation)
            .await
            .map_err(|_| BrokerFailure::Timeout)?
    }

    async fn sqs(
        &self,
        request: &BrokerRequest,
        queue_url: &TargetUrl,
        region: &str,
        credentials: &Credentials,
    ) -> Result<(), BrokerFailure> {
        let client = self
            .clients
            .client(request.tenant_id, request.namespace_id)
            .await
            .map_err(|failure| match failure {
                SendFailure::Denied => BrokerFailure::Denied,
                SendFailure::Timeout => BrokerFailure::Timeout,
                SendFailure::Invalid => BrokerFailure::Rejected,
                SendFailure::Network => BrokerFailure::Network,
            })?;
        let mut parameters = vec![
            pair("Action", "SendMessage"),
            pair("Version", "2012-11-05"),
            pair("QueueUrl", queue_url.as_str()),
            pair("MessageBody", String::from_utf8_lossy(&request.body)),
        ];
        for (index, (name, value)) in request.headers.iter().enumerate() {
            let index = index + 1;
            parameters.push(pair(&format!("MessageAttribute.{index}.Name"), name));
            parameters.push(pair(
                &format!("MessageAttribute.{index}.Value.DataType"),
                "String",
            ));
            parameters.push(pair(
                &format!("MessageAttribute.{index}.Value.StringValue"),
                value,
            ));
        }
        if queue_url.as_str().ends_with(".fifo") {
            parameters.push(pair("MessageGroupId", request.subscription_id));
            parameters.push(pair(
                "MessageDeduplicationId",
                header(request, "delivery-id").ok_or(BrokerFailure::Rejected)?,
            ));
        }
        tokio::time::timeout(
            request.timeout,
            crate::sqs::send_message(
                &client,
                queue_url.as_str(),
                region,
                credentials,
                &parameters,
            ),
        )
        .await
        .map_err(|_| BrokerFailure::Timeout)?
        .map_err(|_| BrokerFailure::Rejected)
    }
}

fn header<'a>(request: &'a BrokerRequest, wanted: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(name, _)| name == wanted)
        .map(|(_, value)| value.as_str())
}

fn pair(name: &str, value: impl ToString) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// A fixed hash so an unchanged Kafka partition count selects the same partition across processes.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::fnv1a;

    #[test]
    fn stable_partition_hash() {
        assert_eq!(fnv1a(b"delivery-123"), fnv1a(b"delivery-123"));
        assert_ne!(fnv1a(b"delivery-123"), fnv1a(b"delivery-124"));
    }
}

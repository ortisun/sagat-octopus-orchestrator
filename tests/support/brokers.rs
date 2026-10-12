//! Real brokers in containers for the connector stories: Kafka, an SQS-compatible queue, and the
//! supervisor that reads them (given a guard that lets it reach this machine, which no policy can).
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rskafka::client::ClientBuilder;
use rskafka::client::partition::{Compression, UnknownTopicHandling};
use rskafka::record::Record;
use sagat_application::connectors::ConnectorRecord;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::connectors::{ConnectorId, ConnectorSettings, ConnectorStatus};
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use sagat_infra_connectors::guard::Unguarded;
use sagat_infra_connectors::supervisor::Supervisor;
use sagat_infra_connectors::webhook::Clients;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use testcontainers_modules::elasticmq::ElasticMq;
use testcontainers_modules::kafka::apache::{KAFKA_PORT, Kafka};
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::now;
use super::receiver::OpenFactory;

pub type TestSupervisor = Supervisor<PgRunStore, Unguarded, PgRunStore, OpenFactory>;

/// What a test says about a connector it stores.
pub struct Wanted<'a> {
    pub slug: &'a str,
    pub settings: ConnectorSettings,
    pub secret: Option<&'a str>,
    pub default_type: Option<&'a str>,
    pub rate: u32,
}

/// A connector stored directly: its brokers are on this machine, which the egress policy that the
/// service applies would refuse.
pub async fn store_connector(
    store: &PgRunStore,
    tenant: TenantId,
    namespace: NamespaceId,
    wanted: Wanted<'_>,
) -> ConnectorRecord {
    use sagat_application::connectors::{ConnectorHealth, ConnectorStore};
    let created = now();
    let record = ConnectorRecord {
        id: ConnectorId::generate(),
        tenant_id: tenant,
        namespace_id: namespace,
        slug: Slug::parse(wanted.slug).expect("slug"),
        settings: wanted.settings,
        secret_name: wanted.secret.map(|s| SecretName::parse(s).expect("secret")),
        default_event_type: wanted
            .default_type
            .map(|t| EventType::parse(t).expect("type")),
        rate_per_second: wanted.rate,
        status: ConnectorStatus::Active,
        health: ConnectorHealth::default(),
        created_by: "test".to_owned(),
        created_at: created,
        updated_at: created,
    };
    let audit = AuditRecord::provisioning(
        tenant,
        AuditAction::ConnectorCreated,
        ResourceType::Connector,
        record.id.to_string(),
        "test",
        created,
    );
    store
        .create_connector(&record, 100, &audit)
        .await
        .expect("store the connector");
    record
}

/// A supervisor of this machine's brokers over `store`.
pub fn supervisor(
    store: &PgRunStore,
    cipher: Arc<dyn sagat_domain::secrets::SecretCipher>,
    holder: &str,
) -> Arc<TestSupervisor> {
    Arc::new(Supervisor::new(
        Arc::new(store.clone()),
        Unguarded,
        Clients::with_factory(store.clone(), OpenFactory),
        cipher,
        Arc::new(JsonEvaluator),
        holder.to_owned(),
        Duration::from_millis(200),
    ))
}

/// A supervisor that is running.
pub struct Running {
    stop: watch::Sender<bool>,
    handle: Option<JoinHandle<()>>,
}

impl Running {
    pub fn start(supervisor: Arc<TestSupervisor>) -> Self {
        let (stop, quit) = watch::channel(false);
        Self {
            stop,
            handle: Some(tokio::spawn(supervisor.run(quit))),
        }
    }

    pub async fn stop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(handle) = self.handle.take() {
            tokio::time::timeout(Duration::from_secs(30), handle)
                .await
                .expect("the supervisor stops")
                .expect("no panic");
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

// ------------------------------------------------------------------------------------- Kafka

pub struct TestKafka {
    _container: ContainerAsync<Kafka>,
    pub bootstrap: String,
}

pub async fn kafka() -> TestKafka {
    let container = Kafka::default()
        .start()
        .await
        .expect("start Kafka (is Docker running?)");
    let port = container
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("mapped Kafka port");
    TestKafka {
        _container: container,
        bootstrap: format!("127.0.0.1:{port}"),
    }
}

/// A record with the headers a producer sets.
pub fn record(headers: &[(&str, &str)], value: &str) -> Record {
    Record {
        key: None,
        value: Some(value.as_bytes().to_vec()),
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.as_bytes().to_vec()))
            .collect::<BTreeMap<_, _>>(),
        timestamp: chrono::Utc::now(),
    }
}

impl TestKafka {
    pub async fn create_topic(&self, topic: &str, partitions: i32) {
        let client = ClientBuilder::new(vec![self.bootstrap.clone()])
            .build()
            .await
            .expect("kafka client");
        client
            .controller_client()
            .expect("controller")
            .create_topic(topic, partitions, 1, 10_000)
            .await
            .expect("create the topic");
    }

    /// Produces `records` to one partition; returns the offsets they got.
    pub async fn produce(&self, topic: &str, partition: i32, records: Vec<Record>) -> Vec<i64> {
        let client = ClientBuilder::new(vec![self.bootstrap.clone()])
            .build()
            .await
            .expect("kafka client");
        let partition = client
            .partition_client(topic.to_owned(), partition, UnknownTopicHandling::Retry)
            .await
            .expect("partition client");
        partition
            .produce(records, Compression::NoCompression)
            .await
            .expect("produce")
    }
}

// ------------------------------------------------------------------------------------- SQS

pub struct TestSqs {
    _container: ContainerAsync<ElasticMq>,
    pub endpoint: String,
    client: reqwest::Client,
}

pub async fn sqs() -> TestSqs {
    sagat_api_http::auth::install_crypto_provider();
    let container = ElasticMq::default()
        .start()
        .await
        .expect("start ElasticMQ (is Docker running?)");
    let port = container
        .get_host_port_ipv4(9324)
        .await
        .expect("mapped SQS port");
    TestSqs {
        _container: container,
        endpoint: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client"),
    }
}

impl TestSqs {
    async fn call(&self, path: &str, parameters: &[(&str, String)]) -> String {
        let response = self
            .client
            .post(format!("{}{path}", self.endpoint))
            .form(parameters)
            .send()
            .await
            .expect("send");
        let status = response.status();
        let text = response.text().await.expect("text");
        assert!(status.is_success(), "SQS answered {status}: {text}");
        text
    }

    /// Creates a queue with a short visibility timeout and returns its URL.
    pub async fn create_queue(&self, name: &str) -> String {
        self.create_queue_with(name, false).await
    }

    /// Creates a FIFO queue when `fifo` is true, retaining the `.fifo` suffix in its URL.
    pub async fn create_queue_with(&self, name: &str, fifo: bool) -> String {
        let mut parameters = vec![
            ("Action", "CreateQueue".to_owned()),
            ("QueueName", name.to_owned()),
        ];
        if fifo {
            parameters.push(("Attribute.1.Name", "FifoQueue".to_owned()));
            parameters.push(("Attribute.1.Value", "true".to_owned()));
        }
        let text = self.call("/", &parameters).await;
        let url = text
            .split("<QueueUrl>")
            .nth(1)
            .and_then(|rest| rest.split("</QueueUrl>").next())
            .expect("the queue url")
            .to_owned();
        // The URL the service reports names the host it believes it is on; the test reaches it
        // through the mapped port.
        let path = url
            .split_once("://")
            .map_or("", |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
        format!("{}/{path}", self.endpoint)
    }

    /// Receives one message, including all string attributes, as the service's XML response.
    pub async fn receive_raw(&self, queue_url: &str) -> String {
        let path = queue_url.strip_prefix(&self.endpoint).unwrap_or("/");
        self.call(
            path,
            &[
                ("Action", "ReceiveMessage".to_owned()),
                ("MaxNumberOfMessages", "1".to_owned()),
                ("MessageAttributeName.1", "All".to_owned()),
            ],
        )
        .await
    }

    pub async fn send(&self, queue_url: &str, body: &str, attributes: &[(&str, &str)]) {
        let path = queue_url.strip_prefix(&self.endpoint).unwrap_or("/");
        let mut parameters = vec![
            ("Action", "SendMessage".to_owned()),
            ("MessageBody", body.to_owned()),
        ];
        for (index, (name, value)) in attributes.iter().enumerate() {
            let n = index + 1;
            parameters.push((
                leak(format!("MessageAttribute.{n}.Name")),
                (*name).to_owned(),
            ));
            parameters.push((
                leak(format!("MessageAttribute.{n}.Value.DataType")),
                "String".to_owned(),
            ));
            parameters.push((
                leak(format!("MessageAttribute.{n}.Value.StringValue")),
                (*value).to_owned(),
            ));
        }
        self.call(path, &parameters).await;
    }

    /// `(visible, in flight)` messages of the queue.
    pub async fn depth(&self, queue_url: &str) -> (u32, u32) {
        let path = queue_url.strip_prefix(&self.endpoint).unwrap_or("/");
        let text = self
            .call(
                path,
                &[
                    ("Action", "GetQueueAttributes".to_owned()),
                    ("AttributeName.1", "All".to_owned()),
                ],
            )
            .await;
        let value = |name: &str| -> u32 {
            text.split(&format!("<Name>{name}</Name>"))
                .nth(1)
                .and_then(|rest| rest.split("<Value>").nth(1))
                .and_then(|rest| rest.split("</Value>").next())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        (
            value("ApproximateNumberOfMessages"),
            value("ApproximateNumberOfMessagesNotVisible"),
        )
    }
}

/// `&'static str` for a parameter name built at run time: a test, a handful of them.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

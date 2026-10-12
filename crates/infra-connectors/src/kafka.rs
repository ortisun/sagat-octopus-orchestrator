//! The Kafka consumer (`FR-004-005`, US03).
//!
//! Kafka's own consumer groups are not used. The position of each partition is kept in the
//! platform's database and moves in the transaction that admits the message, so there is no commit
//! that can be lost between the message and its offset: the edge case "the offset commit is lost"
//! cannot arise, and a partition read from the offset the database holds delivers every message
//! exactly once to the inbox (and the inbox deduplicates by `topic/partition/offset` besides).
//! Replicas share the partitions through a lease on each, so one reads a partition at a time.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rskafka::client::partition::{OffsetAt, PartitionClient, UnknownTopicHandling};
use rskafka::client::{ClientBuilder, Credentials as SaslCredentials, SaslConfig};
use sagat_application::connectors::{Checkpoint, ConnectorRuntime, RawMessage};
use sagat_domain::connectors::{ConnectorSettings, StartFrom};
use tokio::sync::watch;

use crate::credentials::Credentials;
use crate::ingest::{Ingest, Outcome, Pacer};
use crate::nats::Stopped;

/// How long a lease on a partition lasts without being renewed.
const LEASE: Duration = Duration::from_secs(30);
/// The most partitions one connector reads. A topic with more is refused: every partition is a task,
/// a connection and a lease, on every replica, and the topic belongs to whoever runs the broker.
const MAX_PARTITIONS: usize = 256;

/// How long the leases on partitions last. A reader renews its lease at a third of it, between two
/// messages as well as between two fetches, so a long batch that is paced does not outlast it.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub lease: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { lease: LEASE }
    }
}

/// Whether a topic has more partitions than a connector reads.
const fn too_many(partitions: usize) -> bool {
    partitions > MAX_PARTITIONS
}
/// How long a fetch waits for messages.
const FETCH_WAIT_MS: i32 = 500;
/// Fetch size: at least one byte, at most a megabyte, per request.
const FETCH_BYTES: std::ops::Range<i32> = 1..1_000_000;
/// How often the partitions of the topic are listed again.
const REFRESH: Duration = Duration::from_secs(30);
/// How long to wait before reading again after the tenant's backlog was full.
const BACKLOG_PAUSE: Duration = Duration::from_secs(5);

pub(crate) fn sasl(credentials: &Credentials) -> Result<Option<SaslConfig>, &'static str> {
    let (Some(user), Some(password)) = (credentials.text("username"), credentials.text("password"))
    else {
        return Ok(None);
    };
    let pair = SaslCredentials::new(user.to_owned(), password.to_owned());
    Ok(Some(
        match credentials.text("mechanism").unwrap_or("PLAIN") {
            "PLAIN" => SaslConfig::Plain(pair),
            "SCRAM-SHA-256" => SaslConfig::ScramSha256(pair),
            "SCRAM-SHA-512" => SaslConfig::ScramSha512(pair),
            _ => return Err("the Kafka SASL mechanism is not supported"),
        },
    ))
}

pub(crate) fn tls() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
        .unwrap_or_else(|_| {
            rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth()
        }),
    )
}

/// Reads the topic until told to stop or until something fails.
pub async fn run<R: ConnectorRuntime + 'static>(
    ingest: Arc<Ingest<R>>,
    runtime: Arc<R>,
    credentials: Credentials,
    holder: String,
    stop: watch::Receiver<bool>,
) -> Stopped {
    run_with(
        ingest,
        runtime,
        credentials,
        holder,
        stop,
        Timing::default(),
    )
    .await
}

/// As [`run`], with the length of the leases given.
pub async fn run_with<R: ConnectorRuntime + 'static>(
    ingest: Arc<Ingest<R>>,
    runtime: Arc<R>,
    credentials: Credentials,
    holder: String,
    mut stop: watch::Receiver<bool>,
    timing: Timing,
) -> Stopped {
    let connector = ingest.connector();
    let ConnectorSettings::Kafka {
        servers,
        topic,
        start_from,
    } = &connector.settings
    else {
        return Stopped::Failed("the connector is not a Kafka connector");
    };
    let mut builder = ClientBuilder::new(servers.iter().map(|s| s.address()).collect());
    match sasl(&credentials) {
        Ok(Some(config)) => {
            // A password on a connection nobody encrypts is a password given to the network.
            if !servers.iter().all(sagat_domain::connectors::Server::tls) {
                return Stopped::Failed(
                    "credentials are not sent to a broker without TLS (use a tls:// address)",
                );
            }
            builder = builder.sasl_config(config);
        }
        Ok(None) => {}
        Err(why) => return Stopped::Failed(why),
    }
    if servers.iter().any(sagat_domain::connectors::Server::tls) {
        builder = builder.tls_config(tls());
    }
    let Ok(client) = builder.build().await else {
        return Stopped::Failed("could not connect to the Kafka brokers");
    };
    let client = Arc::new(client);
    let mut readers: BTreeMap<i32, tokio::task::JoinHandle<Stopped>> = BTreeMap::new();
    let (quit_tx, quit_rx) = watch::channel(false);
    let result = loop {
        // The partitions of the topic, listed again now and then: a partition added later is read.
        match client.list_topics().await {
            Ok(topics) => {
                let Some(found) = topics.iter().find(|t| &t.name == topic) else {
                    break Stopped::Failed("the Kafka topic does not exist");
                };
                if too_many(found.partitions.len()) {
                    break Stopped::Failed(
                        "the Kafka topic has more partitions than a connector reads",
                    );
                }
                for partition in found.partitions.iter().copied() {
                    if readers.get(&partition).is_some_and(|h| !h.is_finished()) {
                        continue;
                    }
                    let Ok(reader) = client
                        .partition_client(topic.clone(), partition, UnknownTopicHandling::Retry)
                        .await
                    else {
                        continue;
                    };
                    readers.insert(
                        partition,
                        tokio::spawn(read_partition(
                            Arc::clone(&ingest),
                            Arc::clone(&runtime),
                            reader,
                            *start_from,
                            holder.clone(),
                            quit_rx.clone(),
                            timing,
                        )),
                    );
                }
            }
            Err(_) => break Stopped::Failed("could not list the Kafka topic"),
        }
        tokio::select! {
            () = crate::supervisor::stopped(&mut stop) => break Stopped::Asked,
            () = tokio::time::sleep(REFRESH) => {}
        }
    };
    let _ = quit_tx.send(true);
    for (_, reader) in readers {
        let _ = reader.await;
    }
    ingest.flush(true).await;
    result
}

/// Reads one partition: takes its lease, starts where the database says, and moves the position
/// with every message.
async fn read_partition<R: ConnectorRuntime>(
    ingest: Arc<Ingest<R>>,
    runtime: Arc<R>,
    reader: PartitionClient,
    start_from: StartFrom,
    holder: String,
    mut quit: watch::Receiver<bool>,
    timing: Timing,
) -> Stopped {
    let lease = timing.lease;
    let connector = ingest.connector();
    let stream = format!("{}/{}", reader.topic(), reader.partition());
    let mut pacer = Pacer::new(connector.rate_per_second);
    // Where the database says this partition is (what a move of it must start from), and where this
    // reader reads next. They differ only until the first message after a reset.
    let mut stored: Option<i64> = None;
    let mut cursor: Option<i64> = None;
    let mut last_lease = tokio::time::Instant::now() - lease;
    loop {
        if *quit.borrow() {
            return Stopped::Asked;
        }
        // Only the holder of the lease reads the partition.
        if last_lease.elapsed() >= lease / 3 {
            match runtime.lease(connector.id, &stream, &holder, lease).await {
                Ok(true) => last_lease = tokio::time::Instant::now(),
                Ok(false) => {
                    tokio::select! {
                        () = crate::supervisor::stopped(&mut quit) => return Stopped::Asked,
                        () = tokio::time::sleep(Duration::from_secs(5)) => {}
                    }
                    cursor = None;
                    stored = None;
                    continue;
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }
        }
        let position = match cursor {
            Some(position) => position,
            None => {
                stored = runtime
                    .checkpoint(connector.id, &stream)
                    .await
                    .ok()
                    .flatten();
                let position = match stored {
                    Some(offset) => offset,
                    None => {
                        let at = match start_from {
                            StartFrom::Earliest => OffsetAt::Earliest,
                            StartFrom::Latest => OffsetAt::Latest,
                        };
                        match reader.get_offset(at).await {
                            Ok(offset) => offset,
                            Err(_) => return Stopped::Failed("could not read the Kafka offsets"),
                        }
                    }
                };
                cursor = Some(position);
                position
            }
        };
        let fetched = tokio::select! {
            () = crate::supervisor::stopped(&mut quit) => return Stopped::Asked,
            fetched = reader.fetch_records(position, FETCH_BYTES, FETCH_WAIT_MS) => fetched,
        };
        // What has been read is written down about once a second, even when the topic is quiet.
        ingest.flush(false).await;
        let records = match fetched {
            Ok((records, _watermark)) => records,
            Err(rskafka::client::error::Error::ServerError {
                protocol_error: rskafka::client::error::ProtocolError::OffsetOutOfRange,
                ..
            }) => {
                // Retention removed what this reader had not read yet: continue from what is left,
                // and say so, because messages were lost to the broker's retention and not to us.
                tracing::warn!(connector = %connector.id, stream, "the Kafka offset is out of range; starting from the earliest");
                metrics::counter!("connector_offset_resets_total", "kind" => "KAFKA").increment(1);
                match reader.get_offset(OffsetAt::Earliest).await {
                    Ok(offset) => {
                        cursor = Some(offset);
                        continue;
                    }
                    Err(_) => return Stopped::Failed("could not read the Kafka offsets"),
                }
            }
            Err(_) => return Stopped::Failed("could not read the Kafka topic"),
        };
        for record in records {
            if record.offset < position {
                continue;
            }
            pacer.wait().await;
            // A batch that is paced can take longer than a lease: it is renewed between messages,
            // and a reader that can no longer renew it stops and starts over from the database.
            if last_lease.elapsed() >= lease / 3 {
                match runtime.lease(connector.id, &stream, &holder, lease).await {
                    Ok(true) => last_lease = tokio::time::Instant::now(),
                    Ok(false) => {
                        cursor = None;
                        stored = None;
                        break;
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        break;
                    }
                }
            }
            let id = format!(
                "{}/{}/{}",
                reader.topic(),
                reader.partition(),
                record.offset
            );
            let headers = record
                .record
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned()))
                .collect();
            let raw = RawMessage {
                position: id.clone(),
                fallback_id: id,
                headers,
                body: record.record.value.clone().unwrap_or_default(),
            };
            let next = Checkpoint {
                stream: stream.clone(),
                next_offset: record.offset + 1,
            };
            // The first message after the position is first read has no stored row to move from.
            let from = stored.unwrap_or(position);
            match ingest.ingest(&raw, Some((&next, from))).await {
                Outcome::Admitted | Outcome::Duplicate | Outcome::SetAside(_) => {
                    stored = Some(next.next_offset);
                    cursor = Some(next.next_offset);
                }
                Outcome::Backlogged => {
                    // Leave the rest with Kafka and look again later: nothing is lost.
                    tokio::select! {
                        () = crate::supervisor::stopped(&mut quit) => return Stopped::Asked,
                        () = tokio::time::sleep(BACKLOG_PAUSE) => {}
                    }
                    break;
                }
                Outcome::Superseded => {
                    // Another reader moved this partition (our lease ran out): start over from
                    // wherever the database says it is.
                    cursor = None;
                    stored = None;
                    break;
                }
                Outcome::Failed => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_topic_is_read_up_to_a_number_of_partitions_and_no_further() {
        assert!(!too_many(1));
        assert!(!too_many(MAX_PARTITIONS));
        assert!(too_many(MAX_PARTITIONS + 1));
    }
}

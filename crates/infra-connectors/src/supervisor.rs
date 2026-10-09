//! Keeps one consumer running for every active connector (`FR-004-005`).
//!
//! The supervisor looks at the connectors now and then, starts a consumer for each one that has
//! none, stops those whose connector was paused, deleted or changed, and restarts one that failed
//! after a pause that grows to a minute. Every replica of the engine runs one; what keeps them from
//! reading the same thing twice is in the consumers (a durable consumer, a queue, a lease on every
//! partition), so the supervisor is the same everywhere.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sagat_application::connectors::{ConnectorRecord, ConnectorRuntime};
use sagat_application::egress::EgressStore;
use sagat_application::execution::ExpressionEvaluator;
use sagat_domain::connectors::{ConnectorId, ConnectorKind};
use sagat_domain::secrets::SecretCipher;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::credentials::{self, CredentialsError};
use crate::guard::BrokerGuard;
use crate::ingest::Ingest;
use crate::nats::Stopped;
use crate::webhook::{ClientFactory, Clients};

/// Resolves when the stop flag is set.
pub async fn stopped(stop: &mut watch::Receiver<bool>) {
    while !*stop.borrow_and_update() {
        if stop.changed().await.is_err() {
            return;
        }
    }
}

/// How long a consumer is given to stop once it has been told to. One that is stuck in a call that
/// does not look at the flag (a database, a broker that does not answer) is abandoned after that,
/// so it cannot hold up the starting, stopping and refreshing of every other connector.
const WIND_DOWN: Duration = Duration::from_secs(30);

struct Running {
    /// What the consumer was started for: if the connector changed, it is replaced.
    version: OffsetDateTime,
    quit: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Tells a consumer to stop and waits for it for `limit`, then abandons it.
async fn wind_down(mut running: Running, limit: Duration) {
    let _ = running.quit.send(true);
    if tokio::time::timeout(limit, &mut running.handle)
        .await
        .is_err()
    {
        tracing::warn!("a connector's consumer did not stop in time and was abandoned");
        metrics::counter!("connector_consumers_abandoned_total").increment(1);
        running.handle.abort();
    }
}

/// The supervisor of the consumers of a replica.
pub struct Supervisor<R, G, S, F> {
    runtime: Arc<R>,
    guard: G,
    clients: Clients<S, F>,
    cipher: Arc<dyn SecretCipher>,
    evaluator: Arc<dyn ExpressionEvaluator>,
    /// Who this replica is, for the leases on partitions.
    holder: String,
    /// How often the connectors are looked at.
    refresh: Duration,
}

impl<R, G, S, F> Supervisor<R, G, S, F>
where
    R: ConnectorRuntime + 'static,
    G: BrokerGuard + 'static,
    S: EgressStore + 'static,
    F: ClientFactory + 'static,
{
    #[must_use]
    pub fn new(
        runtime: Arc<R>,
        guard: G,
        clients: Clients<S, F>,
        cipher: Arc<dyn SecretCipher>,
        evaluator: Arc<dyn ExpressionEvaluator>,
        holder: String,
        refresh: Duration,
    ) -> Self {
        Self {
            runtime,
            guard,
            clients,
            cipher,
            evaluator,
            holder,
            refresh,
        }
    }

    /// Runs until `stop` is set; then stops every consumer and waits for it.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut running: HashMap<ConnectorId, Running> = HashMap::new();
        loop {
            match self.runtime.active_connectors().await {
                Ok(active) => {
                    let wanted: HashMap<ConnectorId, &ConnectorRecord> =
                        active.iter().map(|c| (c.id, c)).collect();
                    // Stop what is not wanted any more, or is wanted differently.
                    let gone: Vec<ConnectorId> = running
                        .iter()
                        .filter(|(id, r)| {
                            wanted
                                .get(id)
                                .is_none_or(|c| c.updated_at != r.version || r.handle.is_finished())
                        })
                        .map(|(id, _)| *id)
                        .collect();
                    for id in gone {
                        if let Some(r) = running.remove(&id) {
                            wind_down(r, WIND_DOWN).await;
                        }
                    }
                    for connector in active {
                        if running.contains_key(&connector.id) {
                            continue;
                        }
                        let (quit, quit_rx) = watch::channel(false);
                        let version = connector.updated_at;
                        let id = connector.id;
                        let handle =
                            tokio::spawn(Arc::clone(&self).keep_reading(connector, quit_rx));
                        running.insert(
                            id,
                            Running {
                                version,
                                quit,
                                handle,
                            },
                        );
                    }
                    #[allow(clippy::cast_precision_loss)] // a gauge
                    metrics::gauge!("connectors_running").set(running.len() as f64);
                }
                Err(error) => tracing::warn!(%error, "listing the connectors failed"),
            }
            tokio::select! {
                () = stopped(&mut stop) => break,
                () = tokio::time::sleep(self.refresh) => {}
            }
        }
        // Every consumer is told at once and given the same time, not one after the other.
        futures_util::future::join_all(running.into_values().map(|r| wind_down(r, WIND_DOWN)))
            .await;
    }

    /// Reads one connector for as long as it should be read, starting again when reading fails.
    async fn keep_reading(
        self: Arc<Self>,
        connector: ConnectorRecord,
        mut quit: watch::Receiver<bool>,
    ) {
        let ingest = Arc::new(Ingest::new(
            Arc::clone(&self.runtime),
            connector.clone(),
            Arc::clone(&self.evaluator),
        ));
        let mut pause = Duration::from_secs(1);
        loop {
            if *quit.borrow() {
                return;
            }
            let started = tokio::time::Instant::now();
            match self.read_once(&ingest, quit.clone()).await {
                Stopped::Asked => return,
                Stopped::Failed(class) => {
                    tracing::warn!(connector = %connector.id, class, "a connector stopped reading");
                    metrics::counter!("connector_failures_total", "kind" => connector.settings.kind().as_str()).increment(1);
                    ingest.failed(class).await;
                    // A consumer that read for a while before it failed starts over quickly; one that
                    // fails at once waits longer each time.
                    pause = if started.elapsed() > Duration::from_secs(60) {
                        Duration::from_secs(1)
                    } else {
                        (pause * 2).min(Duration::from_secs(60))
                    };
                    tokio::select! {
                        () = stopped(&mut quit) => return,
                        () = tokio::time::sleep(pause) => {}
                    }
                }
            }
        }
    }

    async fn read_once(&self, ingest: &Arc<Ingest<R>>, quit: watch::Receiver<bool>) -> Stopped {
        let connector = ingest.connector();
        let credentials =
            match credentials::open(self.runtime.as_ref(), &self.cipher, connector).await {
                Ok(credentials) => credentials,
                Err(CredentialsError::Store) => {
                    return Stopped::Failed("the database could not be reached");
                }
                Err(CredentialsError::Unusable) => {
                    return Stopped::Failed("the credentials of the connector cannot be used");
                }
            };
        match connector.settings.kind() {
            ConnectorKind::Sqs => {
                // The queue is reached over HTTP through the guarded client of the namespace.
                let Ok(client) = self
                    .clients
                    .client(connector.tenant_id, connector.namespace_id)
                    .await
                else {
                    return Stopped::Failed("the egress client could not be had");
                };
                crate::sqs::run(Arc::clone(ingest), client, credentials, quit).await
            }
            kind => {
                // A library opens its own sockets: the policy is applied before it does.
                if !self
                    .guard
                    .permits(
                        connector.tenant_id,
                        connector.namespace_id,
                        &connector.settings.servers(),
                    )
                    .await
                {
                    return Stopped::Failed(
                        "the egress policy of the namespace does not allow a broker of the connector",
                    );
                }
                match kind {
                    ConnectorKind::Nats => {
                        crate::nats::run(Arc::clone(ingest), credentials, quit).await
                    }
                    _ => {
                        crate::kafka::run(
                            Arc::clone(ingest),
                            Arc::clone(&self.runtime),
                            credentials,
                            self.holder.clone(),
                            quit,
                        )
                        .await
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(handle: JoinHandle<()>) -> Running {
        let (quit, _) = watch::channel(false);
        Running {
            version: OffsetDateTime::UNIX_EPOCH,
            quit,
            handle,
        }
    }

    #[tokio::test]
    async fn a_consumer_that_ignores_the_stop_is_abandoned_after_the_time_it_was_given() {
        let stuck = tokio::spawn(std::future::pending::<()>());
        let abort = stuck.abort_handle();
        let started = std::time::Instant::now();
        wind_down(running(stuck), Duration::from_millis(50)).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it did not wait for ever"
        );
        tokio::task::yield_now().await;
        assert!(abort.is_finished(), "and it was cut off, not left running");
    }

    #[tokio::test]
    async fn a_consumer_that_stops_when_told_is_not_cut_off() {
        let (quit, mut flag) = watch::channel(false);
        let handle = tokio::spawn(async move {
            stopped(&mut flag).await;
        });
        let started = std::time::Instant::now();
        wind_down(
            Running {
                version: OffsetDateTime::UNIX_EPOCH,
                quit,
                handle,
            },
            Duration::from_secs(10),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

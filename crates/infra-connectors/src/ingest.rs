//! What every consumer does with a message, whatever broker it came from (`FR-004-005`).
//!
//! The order is the contract: the message is judged, and either becomes an event in the inbox or is
//! set aside, and the position moves in the same transaction as that. Only then does the consumer
//! tell the broker it is done with the message. Whatever fails before that leaves the message with
//! the broker, to be delivered again, and the inbox recognises the repeat.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sagat_application::connectors::{
    Checkpoint, ConnectorRecord, ConnectorRuntime, Progress, RawMessage, SetAside, decode,
};
use sagat_application::error::StoreError;
use sagat_application::events::{Admission, BACKLOG_LIMIT, new_event};
use sagat_application::execution::ExpressionEvaluator;
use sagat_domain::connectors::DeadLetter;
use sagat_domain::events::DEFAULT_DEDUPE_TTL;

/// What happened to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It is an event in the inbox now.
    Admitted,
    /// It was an event in the inbox already: a redelivery.
    Duplicate,
    /// It could not become an event and is set aside for a person.
    SetAside(DeadLetter),
    /// The tenant has more events waiting than the platform takes: leave the message with the
    /// broker and come back.
    Backlogged,
    /// The position is not where this consumer left it: another consumer reads this stream now.
    Superseded,
    /// The database could not be reached or refused: leave the message with the broker.
    Failed,
}

impl Outcome {
    /// Whether the broker may be told the message is done.
    #[must_use]
    pub const fn done(self) -> bool {
        matches!(self, Self::Admitted | Self::Duplicate | Self::SetAside(_))
    }
}

/// What reading has shown since it was last written down. Writing it for every message would make
/// the connector's row the hottest row of the database; it is written about once a second.
struct Tally {
    progress: Progress,
    flushed: std::time::Instant,
}

/// The judge of the messages of one connector.
pub struct Ingest<R> {
    runtime: Arc<R>,
    connector: ConnectorRecord,
    evaluator: Arc<dyn ExpressionEvaluator>,
    tally: Mutex<Tally>,
}

impl<R: ConnectorRuntime> Ingest<R> {
    pub fn new(
        runtime: Arc<R>,
        connector: ConnectorRecord,
        evaluator: Arc<dyn ExpressionEvaluator>,
    ) -> Self {
        Self {
            runtime,
            connector,
            evaluator,
            tally: Mutex::new(Tally {
                progress: Progress::default(),
                flushed: std::time::Instant::now(),
            }),
        }
    }

    /// Writes down what has been read, if it is time (or `force`).
    pub async fn flush(&self, force: bool) {
        let progress = {
            let mut tally = self.tally.lock().unwrap_or_else(PoisonError::into_inner);
            let pending = tally.progress != Progress::default();
            if !pending || !(force || tally.flushed.elapsed() >= Duration::from_secs(1)) {
                return;
            }
            tally.flushed = std::time::Instant::now();
            std::mem::take(&mut tally.progress)
        };
        if let Err(error) = self.runtime.progress(self.connector.id, &progress).await {
            tracing::warn!(%error, "could not record the progress of a connector");
        }
    }

    /// Records a failure to read the broker, as a class.
    pub async fn failed(&self, class: &'static str) {
        let progress = Progress {
            error: Some(Some(class.to_owned())),
            ..Progress::default()
        };
        if let Err(error) = self.runtime.progress(self.connector.id, &progress).await {
            tracing::warn!(%error, "could not record the failure of a connector");
        }
    }

    #[must_use]
    pub const fn connector(&self) -> &ConnectorRecord {
        &self.connector
    }

    /// Judges `message` and records the outcome. `checkpoint` is the new position and the one it
    /// must move from, for a broker whose position the platform keeps.
    pub async fn ingest(
        &self,
        message: &RawMessage,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> Outcome {
        let outcome = self.judge(message, checkpoint).await;
        metrics::counter!("connector_messages_total", "kind" => self.connector.settings.kind().as_str(), "outcome" => match outcome {
            Outcome::Admitted => "admitted",
            Outcome::Duplicate => "duplicate",
            Outcome::SetAside(_) => "set_aside",
            Outcome::Backlogged => "backlogged",
            Outcome::Superseded => "superseded",
            Outcome::Failed => "failed",
        })
        .increment(1);
        {
            let mut tally = self.tally.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Some(time::OffsetDateTime::now_utc());
            match outcome {
                Outcome::Admitted | Outcome::Duplicate => {
                    tally.progress.consumed += 1;
                    tally.progress.last_message_at = now;
                    // Reading works: whatever failed before no longer does.
                    tally.progress.error = Some(None);
                }
                Outcome::SetAside(_) => {
                    tally.progress.dead_lettered += 1;
                    tally.progress.last_message_at = now;
                }
                Outcome::Backlogged | Outcome::Superseded | Outcome::Failed => {}
            }
        }
        self.flush(false).await;
        outcome
    }

    async fn judge(&self, message: &RawMessage, checkpoint: Option<(&Checkpoint, i64)>) -> Outcome {
        let decoded = match decode(&self.connector, message, self.evaluator.as_ref()) {
            Ok(decoded) => decoded,
            Err(reason) => return self.set_aside(message, reason, checkpoint).await,
        };
        let event = new_event(
            self.connector.tenant_id,
            self.connector.namespace_id,
            decoded.envelope,
            decoded.occurred_at,
            decoded.body,
            DEFAULT_DEDUPE_TTL,
        );
        match self
            .runtime
            .admit(&self.connector, &event, BACKLOG_LIMIT, checkpoint)
            .await
        {
            Ok(Admission::Accepted) => Outcome::Admitted,
            Ok(Admission::Duplicate) => Outcome::Duplicate,
            Ok(Admission::Reused) => {
                self.set_aside(message, DeadLetter::IdReused, checkpoint)
                    .await
            }
            Ok(Admission::Backlogged) => Outcome::Backlogged,
            Err(StoreError::RevisionConflict { .. }) => Outcome::Superseded,
            Err(error) => {
                tracing::warn!(%error, connector = %self.connector.id, "admitting a message failed");
                Outcome::Failed
            }
        }
    }

    async fn set_aside(
        &self,
        message: &RawMessage,
        reason: DeadLetter,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> Outcome {
        match self
            .runtime
            .set_aside(&self.connector, message, reason, checkpoint)
            .await
        {
            Ok(SetAside::Kept) => Outcome::SetAside(reason),
            Ok(SetAside::Full) => {
                tracing::warn!(
                    connector = %self.connector.id,
                    "the connector keeps as many dead letters as it may; a message was set aside and not kept"
                );
                metrics::counter!("connector_dead_letters_dropped_total", "kind" => self.connector.settings.kind().as_str()).increment(1);
                Outcome::SetAside(reason)
            }
            // The broker delivered it again: it is already where it should be.
            Ok(SetAside::AlreadyKept) => Outcome::Duplicate,
            Err(StoreError::RevisionConflict { .. }) => Outcome::Superseded,
            Err(error) => {
                tracing::warn!(%error, connector = %self.connector.id, "setting a message aside failed");
                Outcome::Failed
            }
        }
    }
}

/// Holds a consumer to the ingest rate of its connector.
pub struct Pacer {
    interval: Duration,
    next: tokio::time::Instant,
    /// How far ahead of its slot a message may go: a short burst, not a standing queue.
    slack: Duration,
}

impl Pacer {
    #[must_use]
    pub fn new(per_second: u32) -> Self {
        let interval = Duration::from_secs_f64(1.0 / f64::from(per_second.max(1)));
        Self {
            interval,
            next: tokio::time::Instant::now(),
            slack: Duration::from_millis(100),
        }
    }

    /// Waits until the next message may be taken.
    pub async fn wait(&mut self) {
        let now = tokio::time::Instant::now();
        let slot = self.next.max(now.checked_sub(self.slack).unwrap_or(now));
        if slot > now + self.slack {
            tokio::time::sleep_until(slot - self.slack).await;
        }
        self.next = slot + self.interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_pacer_holds_the_rate_after_a_short_burst() {
        let mut pacer = Pacer::new(10);
        let started = tokio::time::Instant::now();
        for _ in 0..30 {
            pacer.wait().await;
        }
        let elapsed = started.elapsed();
        // Thirty messages at ten a second take about three seconds, less the burst allowed.
        assert!(
            elapsed >= Duration::from_millis(2_800) && elapsed <= Duration::from_millis(3_100),
            "{elapsed:?}"
        );
    }

    #[test]
    fn only_a_message_that_is_in_or_set_aside_is_done() {
        assert!(Outcome::Admitted.done() && Outcome::Duplicate.done());
        assert!(Outcome::SetAside(DeadLetter::NotAnObject).done());
        for left in [Outcome::Backlogged, Outcome::Superseded, Outcome::Failed] {
            assert!(!left.done(), "{left:?} leaves the message with the broker");
        }
    }
}

//! Diagnostics that deliberately collapse PostgreSQL driver detail into fixed safe labels.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use tokio::task::{JoinError, JoinHandle};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

const POSTGRES_IDENTITY_TARGET: &str = "sagat_infra_postgres::identity";
const CAUSE_COUNT: usize = 5;

#[derive(Clone, Copy)]
enum StorageCause {
    PoolTimeout = 0,
    StatementTimeout = 1,
    Deadlock = 2,
    Serialization = 3,
    Other = 4,
}

impl StorageCause {
    const ALL: [Self; CAUSE_COUNT] = [
        Self::PoolTimeout,
        Self::StatementTimeout,
        Self::Deadlock,
        Self::Serialization,
        Self::Other,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::PoolTimeout => "pool_timeout",
            Self::StatementTimeout => "statement_timeout",
            Self::Deadlock => "deadlock",
            Self::Serialization => "serialization",
            Self::Other => "other",
        }
    }
}

fn classify(detail: &str) -> StorageCause {
    if detail.contains("pool timed out") {
        StorageCause::PoolTimeout
    } else if detail.contains("statement timeout") {
        StorageCause::StatementTimeout
    } else if detail.contains("deadlock detected") {
        StorageCause::Deadlock
    } else if detail.contains("could not serialize") || detail.contains("serialization failure") {
        StorageCause::Serialization
    } else {
        StorageCause::Other
    }
}

#[derive(Default)]
struct DetailVisitor {
    cause: Option<StorageCause>,
}

impl Visit for DetailVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "detail" {
            self.cause = Some(classify(&format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "detail" {
            self.cause = Some(classify(value));
        }
    }
}

/// A tracing layer that records only fixed storage-cause labels and writes no event fields.
#[derive(Clone, Default)]
pub struct SafeStorageDiagnostics {
    counts: Arc<[AtomicU64; CAUSE_COUNT]>,
}

impl SafeStorageDiagnostics {
    /// Installs this non-printing layer before the benchmark creates its [`World`](super::World).
    pub fn install() -> Self {
        let diagnostics = Self::default();
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(diagnostics.clone()),
        )
        .expect("the benchmark owns the tracing subscriber");
        tracing::callsite::rebuild_interest_cache();
        diagnostics
    }

    pub fn reset(&self) {
        for count in self.counts.iter() {
            count.store(0, Ordering::Relaxed);
        }
    }

    pub fn counts(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        StorageCause::ALL.into_iter().filter_map(|cause| {
            let count = self.counts[cause as usize].load(Ordering::Relaxed);
            (count > 0).then_some((cause.label(), count))
        })
    }
}

impl<S> Layer<S> for SafeStorageDiagnostics
where
    S: Subscriber,
{
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        metadata.target() == POSTGRES_IDENTITY_TARGET
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = DetailVisitor::default();
        event.record(&mut visitor);
        if let Some(cause) = visitor.cause {
            self.counts[cause as usize].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// On a failed configuration, cancellation must not wait for an in-flight observer query.
pub async fn finish_sampler<T>(
    sampler: JoinHandle<T>,
    failed: bool,
) -> Result<Option<T>, JoinError> {
    if failed {
        sampler.abort();
        Ok(None)
    } else {
        sampler.await.map(Some)
    }
}

/// Drives workers until all finish or the first failure completes, dropping pending siblings.
pub async fn stop_on_first_error<I, F, T, E>(futures: I) -> Result<Vec<T>, E>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = Result<T, E>>,
{
    let mut pending = futures
        .into_iter()
        .collect::<futures_util::stream::FuturesUnordered<_>>();
    let mut completed = Vec::new();
    while let Some(result) = pending.next().await {
        completed.push(result?);
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::bounded;
    use std::time::Duration;

    #[test]
    fn raw_and_unrecognized_driver_detail_collapses_to_a_static_label() {
        let diagnostics = SafeStorageDiagnostics::default();
        let subscriber = tracing_subscriber::registry().with(diagnostics.clone());
        let sentinel = "SENTINEL postgres://user:password@example.invalid/private?query=secret";
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: POSTGRES_IDENTITY_TARGET,
                detail = %sentinel,
                "storage unavailable"
            );
        });

        let summary = diagnostics.counts().collect::<Vec<_>>();
        assert_eq!(summary, vec![("other", 1)]);
        assert!(
            summary
                .iter()
                .all(|(label, _)| !label.contains("SENTINEL") && !label.contains("password"))
        );
    }

    #[test]
    fn recognized_driver_details_have_only_approved_labels() {
        for (detail, expected) in [
            (
                "pool timed out while waiting for an open connection",
                "pool_timeout",
            ),
            (
                "canceling statement due to statement timeout",
                "statement_timeout",
            ),
            ("deadlock detected", "deadlock"),
            (
                "could not serialize access due to concurrent update",
                "serialization",
            ),
        ] {
            assert_eq!(classify(detail).label(), expected);
        }
    }

    #[tokio::test]
    async fn failed_configuration_aborts_a_pending_sampler_without_waiting() {
        bounded::within(
            "failed performance sampler cancellation",
            Duration::from_secs(5),
            async {
                let barrier = Arc::new(tokio::sync::Barrier::new(2));
                let sampler = tokio::spawn({
                    let barrier = Arc::clone(&barrier);
                    async move {
                        barrier.wait().await;
                        std::future::pending::<()>().await;
                    }
                });
                barrier.wait().await;

                assert!(
                    finish_sampler(sampler, true)
                        .await
                        .expect("abort")
                        .is_none()
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn first_failure_drops_a_pending_sibling_and_retains_its_progress() {
        bounded::within(
            "failed performance worker cancels pending sibling",
            Duration::from_secs(5),
            async {
                let barrier = Arc::new(tokio::sync::Barrier::new(2));
                let mut acknowledged = [Vec::new(), Vec::new()];
                let result = stop_on_first_error(acknowledged.iter_mut().enumerate().map(
                    |(worker, progress)| {
                        let barrier = Arc::clone(&barrier);
                        async move {
                            progress.push(worker);
                            barrier.wait().await;
                            if worker == 0 {
                                Err("worker failure")
                            } else {
                                std::future::pending::<Result<(), &str>>().await
                            }
                        }
                    },
                ))
                .await;

                assert_eq!(result, Err("worker failure"));
                assert_eq!(acknowledged, [vec![0], vec![1]]);
            },
        )
        .await;
    }
}

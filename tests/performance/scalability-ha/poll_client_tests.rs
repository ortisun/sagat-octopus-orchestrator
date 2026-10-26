use super::*;
use crate::support::bounded;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
struct FixedEntropy(f64);

impl Entropy for FixedEntropy {
    fn unit(&self) -> f64 {
        self.0
    }

    fn token(&self) -> [u8; 32] {
        [1; 32]
    }
}

#[tokio::test]
async fn nonretryable_error_returns_after_one_attempt() {
    bounded::within("nonretryable poll error", Duration::from_secs(1), async {
        let attempts = AtomicU32::new(0);
        let mut counters = PollCounters::default();
        let failure = poll_operation(
            PollRetryPolicy::default(),
            &FixedEntropy(0.0),
            &mut counters,
            || async {
                attempts.fetch_add(1, Ordering::Relaxed);
                Err(AppError::StaleAttempt)
            },
        )
        .await
        .expect_err("stale attempt must remain strict");
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(counters.attempts, 1);
        assert_eq!(failure.attempts, 1);
        assert_eq!(failure.retryable_failures, 0);
        assert!(!failure.exhausted);
        assert!(matches!(failure.error, AppError::StaleAttempt));
    })
    .await;
}

#[tokio::test]
async fn unavailable_exhausts_the_configured_attempt_budget() {
    bounded::within(
        "bounded unavailable attempts",
        Duration::from_secs(1),
        async {
            let attempts = AtomicU32::new(0);
            let mut counters = PollCounters::default();
            let failure = poll_operation(
                PollRetryPolicy {
                    initial_delay: Duration::ZERO,
                    max_delay: Duration::ZERO,
                    ..PollRetryPolicy::default()
                },
                &FixedEntropy(0.0),
                &mut counters,
                || async {
                    attempts.fetch_add(1, Ordering::Relaxed);
                    Err(AppError::Unavailable)
                },
            )
            .await
            .expect_err("persistent unavailability must be returned");
            assert_eq!(attempts.load(Ordering::Relaxed), 4);
            assert_eq!(counters.attempts, 4);
            assert_eq!(counters.retryable_failures, 4);
            assert_eq!(counters.exhausted_calls, 1);
            assert_eq!(failure.attempts, 4);
            assert_eq!(failure.retryable_failures, 4);
            assert!(failure.exhausted);
            assert!(matches!(failure.error, AppError::Unavailable));
        },
    )
    .await;
}

#[tokio::test]
async fn retries_are_one_successful_logical_call_sample() {
    bounded::within(
        "successful logical poll accounting",
        Duration::from_secs(1),
        async {
            let attempts = AtomicU32::new(0);
            let mut counters = PollCounters::default();
            let outcome = poll_operation(
                PollRetryPolicy {
                    initial_delay: Duration::ZERO,
                    max_delay: Duration::ZERO,
                    ..PollRetryPolicy::default()
                },
                &FixedEntropy(0.0),
                &mut counters,
                || async {
                    let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
                    if attempt < 3 {
                        Err(AppError::Unavailable)
                    } else {
                        Ok(Vec::new())
                    }
                },
            )
            .await
            .expect("third attempt succeeds");
            assert_eq!(attempts.load(Ordering::Relaxed), 3);
            assert_eq!(counters.attempts, 3);
            assert_eq!(counters.retryable_failures, 2);
            assert_eq!(counters.exhausted_calls, 0);
            assert_eq!(outcome.attempts, 3);
            assert_eq!(outcome.retryable_failures, 2);
            assert!(outcome.tasks.is_empty());
        },
    )
    .await;
}

#[tokio::test]
async fn cancelling_during_backoff_retains_attempt_and_failure_counters() {
    bounded::within(
        "cancel poll during retry backoff",
        Duration::from_secs(1),
        async {
            let (backoff_tx, backoff_rx) = tokio::sync::oneshot::channel();
            let backoff_tx = Arc::new(Mutex::new(Some(backoff_tx)));
            let mut counters = PollCounters::default();
            let mut progress = crate::WorkerProgress::new();
            let mut call = Box::pin(poll_operation_with_sleep(
                PollRetryPolicy::default(),
                &FixedEntropy(1.0),
                &mut counters,
                || async { Err(AppError::Unavailable) },
                move |_| {
                    let backoff_tx = Arc::clone(&backoff_tx);
                    async move {
                        if let Some(sender) = backoff_tx.lock().expect("backoff signal").take() {
                            let _ = sender.send(());
                        }
                        std::future::pending().await
                    }
                },
            ));
            tokio::select! {
                biased;
                signal = backoff_rx => signal.expect("retry backoff entered"),
                result = &mut call => {
                    let outcome = result.expect("call unexpectedly failed before cancellation");
                    progress.record_poll_success(&outcome);
                }
            }
            drop(call);

            assert_eq!(
                counters.attempts, 1,
                "the cancelled attempt must remain accounted"
            );
            assert_eq!(
                counters.retryable_failures, 1,
                "the failure before cancellation must remain accounted"
            );
            assert_eq!(counters.exhausted_calls, 0);
            assert!(
                progress.logical_polls.is_empty(),
                "an aborted call is not a successful logical-call sample"
            );

            let mut success_counters = PollCounters::default();
            let successful = poll_operation(
                PollRetryPolicy::default(),
                &FixedEntropy(0.0),
                &mut success_counters,
                || async { Ok(Vec::new()) },
            )
            .await
            .expect("successful control poll");
            progress.record_poll_success(&successful);
            assert_eq!(progress.logical_polls.len(), 1);
        },
    )
    .await;
}

#[test]
fn retry_delay_uses_sdk_exponential_jitter_and_cap() {
    let policy = PollRetryPolicy::default();
    assert_eq!(
        retry_delay(policy, 1, &FixedEntropy(0.0)),
        Duration::from_millis(100)
    );
    assert_eq!(
        retry_delay(policy, 2, &FixedEntropy(1.0)),
        Duration::from_millis(400)
    );
    assert_eq!(
        retry_delay(policy, 20, &FixedEntropy(1.0)),
        Duration::from_secs(5)
    );
}

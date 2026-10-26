use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_application::entropy::{Entropy, SystemEntropy};
use sagat_application::error::AppError;
use sagat_application::worker::ClaimedTask;
use sqlx::PgPool;

use crate::support::engine::{Replica, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollRetryPolicy {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for PollRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

pub struct PollSuccess {
    pub tasks: Vec<ClaimedTask>,
    pub attempts: u32,
    pub retryable_failures: u32,
    pub success_attempt_elapsed: Duration,
    pub logical_call_elapsed: Duration,
}

pub struct PollFailure {
    pub error: AppError,
    pub attempts: u32,
    pub retryable_failures: u32,
    pub exhausted: bool,
    pub logical_call_elapsed: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollCounters {
    pub attempts: u64,
    pub retryable_failures: u64,
    pub exhausted_calls: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolOracleFailure {
    HandlesOutstanding,
    BackendIdentityUnavailable,
    ObservationUnavailable,
    OpenTransaction,
}

impl PoolOracleFailure {
    pub const fn label(self) -> &'static str {
        match self {
            Self::HandlesOutstanding => "pool_handles_outstanding",
            Self::BackendIdentityUnavailable => "pool_backend_identity_unavailable",
            Self::ObservationUnavailable => "pool_transaction_observation_unavailable",
            Self::OpenTransaction => "pool_open_transaction",
        }
    }
}

async fn ensure_transactions_closed(
    observer: &PgPool,
    backend_pids: &[i32],
) -> Result<(), PoolOracleFailure> {
    let (observed, open): (i64, i64) = sqlx::query_as(
        "SELECT count(*)::bigint, \
                count(*) FILTER (WHERE xact_start IS NOT NULL \
                    OR state = 'idle in transaction (aborted)')::bigint \
           FROM pg_stat_activity WHERE pid = ANY($1)",
    )
    .bind(backend_pids)
    .fetch_one(observer)
    .await
    .map_err(|_| PoolOracleFailure::ObservationUnavailable)?;
    if usize::try_from(observed).unwrap_or(usize::MAX) != backend_pids.len() {
        return Err(PoolOracleFailure::BackendIdentityUnavailable);
    }
    if open == 0 {
        Ok(())
    } else {
        Err(PoolOracleFailure::OpenTransaction)
    }
}

pub async fn verify_replica_transactions(
    observer: &PgPool,
    replicas: &[Arc<Replica>],
) -> Result<(), PoolOracleFailure> {
    verify_replica_transactions_with_limit(observer, replicas, Duration::from_secs(5)).await
}

async fn verify_replica_transactions_with_limit(
    observer: &PgPool,
    replicas: &[Arc<Replica>],
    settle_limit: Duration,
) -> Result<(), PoolOracleFailure> {
    for replica in replicas {
        let mut reclaimed = Vec::new();
        let settle_deadline = Instant::now() + settle_limit;
        loop {
            while let Some(connection) = replica.execution_pool.try_acquire() {
                reclaimed.push(connection);
            }
            let size = usize::try_from(replica.execution_pool.size()).unwrap_or(usize::MAX);
            let idle = replica.execution_pool.num_idle();
            if size == reclaimed.len() && idle == 0 {
                break;
            }
            if Instant::now() >= settle_deadline {
                return Err(PoolOracleFailure::HandlesOutstanding);
            }
            tokio::task::yield_now().await;
        }

        let mut backend_pids = Vec::new();
        for connection in &mut reclaimed {
            backend_pids.push(
                sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut **connection)
                    .await
                    .map_err(|_| PoolOracleFailure::BackendIdentityUnavailable)?,
            );
        }
        let unique = backend_pids
            .iter()
            .collect::<std::collections::HashSet<_>>();
        if unique.len() != reclaimed.len() {
            return Err(PoolOracleFailure::BackendIdentityUnavailable);
        }
        ensure_transactions_closed(observer, &backend_pids).await?;
        if usize::try_from(replica.execution_pool.size()).unwrap_or(usize::MAX) != reclaimed.len()
            || replica.execution_pool.num_idle() != 0
        {
            return Err(PoolOracleFailure::HandlesOutstanding);
        }
        drop(reclaimed);

        let return_deadline = Instant::now() + settle_limit;
        loop {
            let size = usize::try_from(replica.execution_pool.size()).unwrap_or(usize::MAX);
            if size == replica.execution_pool.num_idle() {
                break;
            }
            if Instant::now() >= return_deadline {
                return Err(PoolOracleFailure::HandlesOutstanding);
            }
            tokio::task::yield_now().await;
        }
    }
    Ok(())
}

impl std::fmt::Debug for PollSuccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PollSuccess")
            .field("task_count", &self.tasks.len())
            .field("attempts", &self.attempts)
            .field("retryable_failures", &self.retryable_failures)
            .field("success_attempt_elapsed", &self.success_attempt_elapsed)
            .field("logical_call_elapsed", &self.logical_call_elapsed)
            .finish()
    }
}

impl std::fmt::Debug for PollFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PollFailure")
            .field(
                "error_class",
                &if matches!(&self.error, AppError::Unavailable) {
                    "unavailable"
                } else {
                    "other"
                },
            )
            .field("attempts", &self.attempts)
            .field("retryable_failures", &self.retryable_failures)
            .field("exhausted", &self.exhausted)
            .field("logical_call_elapsed", &self.logical_call_elapsed)
            .finish()
    }
}

pub async fn poll(
    replica: &Replica,
    session: &Session,
    max: u32,
    counters: &mut PollCounters,
) -> Result<PollSuccess, PollFailure> {
    poll_operation(PollRetryPolicy::default(), &SystemEntropy, counters, || {
        replica.workers.poll(
            &session.principal,
            session.worker_id,
            &session.token,
            max,
            None,
        )
    })
    .await
}

async fn poll_operation<E, F, Fut>(
    policy: PollRetryPolicy,
    entropy: &E,
    counters: &mut PollCounters,
    operation: F,
) -> Result<PollSuccess, PollFailure>
where
    E: Entropy,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<ClaimedTask>, AppError>>,
{
    poll_operation_with_sleep(policy, entropy, counters, operation, tokio::time::sleep).await
}

async fn poll_operation_with_sleep<E, F, Fut, S, Sleep>(
    policy: PollRetryPolicy,
    entropy: &E,
    counters: &mut PollCounters,
    mut operation: F,
    mut sleep: S,
) -> Result<PollSuccess, PollFailure>
where
    E: Entropy,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<ClaimedTask>, AppError>>,
    S: FnMut(Duration) -> Sleep,
    Sleep: Future<Output = ()>,
{
    let started = Instant::now();
    let max_attempts = policy.max_attempts.max(1);
    let mut retryable_failures = 0;
    for attempt in 1..=max_attempts {
        counters.attempts += 1;
        let attempt_started = Instant::now();
        match operation().await {
            Ok(tasks) => {
                return Ok(PollSuccess {
                    tasks,
                    attempts: attempt,
                    retryable_failures,
                    success_attempt_elapsed: attempt_started.elapsed(),
                    logical_call_elapsed: started.elapsed(),
                });
            }
            Err(error) if matches!(error, AppError::Unavailable) => {
                retryable_failures += 1;
                counters.retryable_failures += 1;
                if attempt == max_attempts {
                    counters.exhausted_calls += 1;
                    return Err(PollFailure {
                        error,
                        attempts: attempt,
                        retryable_failures,
                        exhausted: true,
                        logical_call_elapsed: started.elapsed(),
                    });
                }
                sleep(retry_delay(policy, attempt, entropy)).await;
            }
            Err(error) => {
                return Err(PollFailure {
                    error,
                    attempts: attempt,
                    retryable_failures,
                    exhausted: false,
                    logical_call_elapsed: started.elapsed(),
                });
            }
        }
    }
    unreachable!("max_attempts is at least one")
}

fn retry_delay<E: Entropy>(policy: PollRetryPolicy, failed_attempt: u32, entropy: &E) -> Duration {
    let exponent = failed_attempt.saturating_sub(1).min(31);
    let base = policy
        .initial_delay
        .saturating_mul(1_u32 << exponent)
        .min(policy.max_delay);
    base.mul_f64(0.5 + 0.5 * entropy.unit().clamp(0.0, 1.0))
}

#[cfg(test)]
#[path = "poll_client_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "poll_client_pg_tests.rs"]
mod pg_tests;

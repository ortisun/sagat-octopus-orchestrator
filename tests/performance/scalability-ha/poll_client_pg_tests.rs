use super::*;
use crate::support::bounded;
use crate::support::engine::{activity, document, task_ref, world};
use sagat_domain::execution::RunStatus;
use serde_json::json;
use sqlx::{Postgres, pool::PoolConnection};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const SCENARIO_LIMIT: Duration = bounded::LIFECYCLE_LIMIT;
const POOL_SIZE: u32 = 20;

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

async fn occupy_pool(replica: &Replica) -> (Vec<PoolConnection<Postgres>>, Vec<i32>) {
    let mut held = Vec::new();
    let mut backend_pids = Vec::new();
    for _ in 0..POOL_SIZE {
        let mut connection = replica
            .execution_pool
            .acquire()
            .await
            .unwrap_or_else(|_| panic!("pool_capacity_probe_acquire_failed"));
        backend_pids.push(
            sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await
                .unwrap_or_else(|_| panic!("pool_backend_identity_probe_failed")),
        );
        held.push(connection);
    }
    assert_eq!(backend_pids.iter().collect::<HashSet<_>>().len(), 20);
    assert_eq!(replica.execution_pool.num_idle(), 0);
    (held, backend_pids)
}

async fn assert_pool_released_without_transactions(
    world: &crate::support::engine::World,
    replica: &Replica,
) {
    let mut reclaimed = Vec::new();
    let mut backend_pids = Vec::new();
    for _ in 0..POOL_SIZE {
        let mut connection = replica
            .execution_pool
            .acquire()
            .await
            .unwrap_or_else(|_| panic!("pool_release_probe_acquire_failed"));
        backend_pids.push(
            sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await
                .unwrap_or_else(|_| panic!("pool_release_identity_probe_failed")),
        );
        reclaimed.push(connection);
    }
    assert_eq!(
        replica.execution_pool.num_idle(),
        0,
        "all execution-pool connections are held by the capacity probe"
    );
    ensure_transactions_closed(&world.db.pool, &backend_pids)
        .await
        .unwrap_or_else(|failure| panic!("pool_release_oracle_failed: {}", failure.label()));
    drop(reclaimed);
}

async fn wait_until_backend_is_aborted(observer: &sqlx::PgPool, backend_pid: i32) {
    bounded::within(
        "observe aborted pool transaction",
        Duration::from_secs(5),
        async {
            loop {
                let observed: Option<(String, bool)> = sqlx::query_as(
                    "SELECT state, xact_start IS NULL FROM pg_stat_activity WHERE pid = $1",
                )
                .bind(backend_pid)
                .fetch_optional(observer)
                .await
                .unwrap_or_else(|_| panic!("aborted_transaction_observation_failed"));
                if observed.as_ref().is_some_and(|(state, xact_is_null)| {
                    state == "idle in transaction (aborted)" && *xact_is_null
                }) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_poll_recovers_after_the_real_pool_is_released() {
    bounded::lifecycle(
        "poll client recovers after real pool exhaustion",
        SCENARIO_LIMIT,
        |progress| async move {
            progress.enter("create real PostgreSQL world");
            let world = world().await;
            let (workflow, _) = world
                .publish(document("poll-recovery", "work", vec![activity("work")]))
                .await;
            let runs = [
                world
                    .start(workflow, &json!({"case": "pool-recovery-1"}))
                    .await,
                world
                    .start(workflow, &json!({"case": "pool-recovery-2"}))
                    .await,
            ];
            let session = world.session(&["act.work"]).await;
            let replica = world.replica_with_pool(POOL_SIZE).await;

            progress.enter("occupy every execution-pool connection");
            let (held, _) = occupy_pool(&replica).await;

            let (failed_tx, failed_rx) = tokio::sync::oneshot::channel();
            let failed_tx = Arc::new(Mutex::new(Some(failed_tx)));
            let workers = &replica.workers;
            let principal = &session.principal;
            let worker_id = session.worker_id;
            let token = &session.token;
            let mut counters = PollCounters::default();
            progress.enter("observe unavailable and release pool");
            let client = poll_operation(
                PollRetryPolicy::default(),
                &FixedEntropy(0.0),
                &mut counters,
                move || {
                    let failed_tx = Arc::clone(&failed_tx);
                    async move {
                        let result = workers.poll(principal, worker_id, token, 2, None).await;
                        if matches!(result, Err(AppError::Unavailable))
                            && let Some(sender) = failed_tx.lock().expect("failure signal").take()
                        {
                            let _ = sender.send(());
                        }
                        result
                    }
                },
            );
            let release = async move {
                failed_rx.await.expect("real unavailable was observed");
                drop(held);
            };
            let (result, ()) = tokio::join!(client, release);

            progress.enter("assert recovered claim");
            let outcome = result.expect("poll should retry after the pool becomes available");
            assert_eq!(outcome.tasks.len(), 2);
            assert_eq!(outcome.attempts, 2);
            assert_eq!(outcome.retryable_failures, 1);
            assert_eq!(counters.attempts, 2);
            assert_eq!(counters.retryable_failures, 1);
            let task_ids = outcome
                .tasks
                .iter()
                .map(ClaimedTask::task_id)
                .collect::<HashSet<_>>();
            assert_eq!(task_ids.len(), 2, "each logical task was claimed once");
            assert_eq!(
                outcome
                    .tasks
                    .iter()
                    .map(|task| task.run_id)
                    .collect::<HashSet<_>>(),
                runs.into_iter().collect::<HashSet<_>>()
            );

            progress.enter("complete recovered claims");
            for task in &outcome.tasks {
                replica
                    .workers
                    .complete(
                        &session.principal,
                        &task_ref(task),
                        b"{}",
                        &format!("done-{}", task.task_id()),
                    )
                    .await
                    .expect("complete recovered claim");
            }

            progress.enter("verify terminal journal and replay");
            for run in runs {
                assert_eq!(world.run_status(run).await, RunStatus::Succeeded);
                let journal = world.journal(run).await;
                assert_eq!(
                    journal
                        .iter()
                        .filter(|event| *event == "step.completed")
                        .count(),
                    1
                );
                assert_eq!(
                    journal
                        .iter()
                        .filter(|event| *event == "run.finished")
                        .count(),
                    1
                );
                world.assert_consistent(run).await;
            }
            let run_ids = runs.map(|run| run.as_uuid());
            let leased: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM steps WHERE run_id = ANY($1) AND status = 'LEASED'",
            )
            .bind(run_ids.as_slice())
            .fetch_one(&world.db.pool)
            .await
            .expect("observe leases");
            assert_eq!(leased, 0, "a recovered run retained a lease");
            assert_pool_released_without_transactions(&world, &replica).await;
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pool_exhaustion_stops_after_four_unavailable_attempts() {
    bounded::lifecycle(
        "persistent real pool exhaustion is bounded",
        SCENARIO_LIMIT,
        |progress| async move {
            progress.enter("create real PostgreSQL world");
            let world = world().await;
            let session = world.session(&["act.work"]).await;
            let replica = world.replica_with_pool(POOL_SIZE).await;

            progress.enter("occupy every execution-pool connection");
            let (held, _) = occupy_pool(&replica).await;

            progress.enter("exhaust poll attempt budget");
            let mut counters = PollCounters::default();
            let failure = poll(&replica, &session, 1, &mut counters)
                .await
                .expect_err("persistent pool exhaustion must be returned");
            assert!(matches!(failure.error, AppError::Unavailable));
            assert_eq!(failure.attempts, 4);
            assert_eq!(failure.retryable_failures, 4);
            assert!(failure.exhausted);
            assert_eq!(counters.attempts, 4);
            assert_eq!(counters.retryable_failures, 4);
            assert_eq!(counters.exhausted_calls, 1);

            progress.enter("release holders and verify no open transaction");
            drop(held);
            assert_pool_released_without_transactions(&world, &replica).await;
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_oracle_rejects_open_and_aborted_pool_transactions() {
    bounded::lifecycle(
        "pool transaction oracle detects open and aborted transactions",
        SCENARIO_LIMIT,
        |progress| async move {
            progress.enter("create real PostgreSQL world");
            let world = world().await;
            let replica = world.replica_with_pool(POOL_SIZE).await;
            let mut open = replica
                .execution_pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("acquire open-transaction connection"));
            let mut aborted = replica
                .execution_pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("acquire aborted-transaction connection"));
            let open_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *open)
                .await
                .unwrap_or_else(|_| panic!("observe open-transaction backend"));
            let aborted_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *aborted)
                .await
                .unwrap_or_else(|_| panic!("observe aborted-transaction backend"));
            sqlx::query("BEGIN")
                .execute(&mut *open)
                .await
                .unwrap_or_else(|_| panic!("open transaction"));
            sqlx::query("BEGIN")
                .execute(&mut *aborted)
                .await
                .unwrap_or_else(|_| panic!("open transaction for abort"));
            assert!(
                sqlx::query("SELECT 1 / 0")
                    .execute(&mut *aborted)
                    .await
                    .is_err(),
                "fixture must abort its transaction"
            );
            wait_until_backend_is_aborted(&world.db.pool, aborted_pid).await;

            progress.enter("observe transaction states from admin pool");
            let pids = [open_pid, aborted_pid];
            let observed = ensure_transactions_closed(&world.db.pool, &pids).await;
            let open_only = ensure_transactions_closed(&world.db.pool, &[open_pid]).await;
            let aborted_only = ensure_transactions_closed(&world.db.pool, &[aborted_pid]).await;

            progress.enter("rollback fixture transactions");
            sqlx::query("ROLLBACK")
                .execute(&mut *open)
                .await
                .unwrap_or_else(|_| panic!("rollback open transaction"));
            sqlx::query("ROLLBACK")
                .execute(&mut *aborted)
                .await
                .unwrap_or_else(|_| panic!("rollback aborted transaction"));

            assert_eq!(observed, Err(PoolOracleFailure::OpenTransaction));
            assert_eq!(open_only, Err(PoolOracleFailure::OpenTransaction));
            assert_eq!(aborted_only, Err(PoolOracleFailure::OpenTransaction));
            assert_eq!(
                ensure_transactions_closed(&world.db.pool, &pids).await,
                Ok(())
            );
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_oracle_checks_existing_connections_without_growing_pool() {
    bounded::lifecycle(
        "pool transaction oracle does not create connections",
        SCENARIO_LIMIT,
        |progress| async move {
            progress.enter("create partially populated real pool");
            let world = world().await;
            let replica = Arc::new(world.replica_with_pool(POOL_SIZE).await);
            let mut created = Vec::new();
            for _ in 0..3 {
                created.push(
                    replica
                        .execution_pool
                        .acquire()
                        .await
                        .unwrap_or_else(|_| panic!("create pool connection")),
                );
            }
            assert_eq!(replica.execution_pool.size(), 3);
            drop(created);

            progress.enter("inspect only existing pool connections");
            verify_replica_transactions_with_limit(
                &world.db.pool,
                &[Arc::clone(&replica)],
                Duration::from_secs(1),
            )
            .await
            .expect("existing connections are clean");
            assert_eq!(
                replica.execution_pool.size(),
                3,
                "oracle must not grow a pool configured for twenty"
            );
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_oracle_rejects_an_externally_retained_pool_handle() {
    bounded::lifecycle(
        "pool transaction oracle detects retained handles",
        SCENARIO_LIMIT,
        |progress| async move {
            progress.enter("create real pool with two connections");
            let world = world().await;
            let replica = Arc::new(world.replica_with_pool(POOL_SIZE).await);
            let first = replica
                .execution_pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("create first pool connection"));
            let second = replica
                .execution_pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("create second pool connection"));
            drop(first);
            drop(second);
            let retained = replica
                .execution_pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("retain pool handle"));

            progress.enter("reject retained handle at bounded deadline");
            let result = verify_replica_transactions_with_limit(
                &world.db.pool,
                &[Arc::clone(&replica)],
                Duration::from_millis(20),
            )
            .await;
            assert_eq!(result, Err(PoolOracleFailure::HandlesOutstanding));

            progress.enter("release handle and prove pool settles");
            drop(retained);
            verify_replica_transactions_with_limit(
                &world.db.pool,
                &[replica],
                Duration::from_secs(1),
            )
            .await
            .expect("released pool is clean");
        },
    )
    .await;
}

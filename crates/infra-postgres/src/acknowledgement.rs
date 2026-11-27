//! The delivery acknowledgement of worker protocol 1.8 in PostgreSQL (feature 006, Task 006-049; Architecture decision 0099
//! decision 3; `specs/006-sdk-multilanguage/stream-completion.md`, "The server's acknowledgement").
//!
//! One transaction per call and about eight statements whatever its size: the settings, `matched`, two for the binding,
//! two for the membership, `lock_matched`, `acknowledge`. It never takes the run lock and never waits for a step row
//! (`SKIP LOCKED`), so it cannot deadlock on rows; the binding and membership locks come before any row lock, as in every
//! other path, so a revocation cannot form a cycle with it. Every statement is checked against the call's deadline before
//! it runs, and the lock and statement timeouts of 1 s bound how far past it a call can end.

use std::time::{Duration, Instant};

use sagat_application::error::StoreError;
use sagat_application::execution::{LeaseDeadlines, LeaseMarks};
use sagat_application::worker::TaskKind;
use sagat_application::worker::acknowledgement::{
    ACKNOWLEDGEMENT_STATEMENT_TIMEOUT, AcknowledgementStore, AcknowledgementStoreError,
    AcknowledgementTx, CallDeadline, LeaseProbe, LockedLease, MatchedLease, WrittenAcknowledgement,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sqlx::Row;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::{PgRunStore, stored_path_parts};
use crate::identity::store_error;
use crate::pool::store_unreachable;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;

/// How long a rollback at the deadline may take before the connection is discarded instead of returned.
const ROLLBACK_BOUND: Duration = Duration::from_millis(100);
/// How long the rollback of a call that was dropped may take, off the call, before its connection is discarded: the
/// statement it was waiting for can take one statement timeout to end (a lock it waits for), and nobody waits for this
/// rollback, so a host that resets its calls does not make the server discard the pool's connections (security review
/// round 2 of Task 006-049).
const DROPPED_ROLLBACK_BOUND: Duration =
    ACKNOWLEDGEMENT_STATEMENT_TIMEOUT.saturating_add(ROLLBACK_BOUND);

fn acknowledgement_error(error: sqlx::Error) -> AcknowledgementStoreError {
    if store_unreachable(&error) {
        tracing::warn!(%error, "the database could not be reached by an acknowledgement");
        AcknowledgementStoreError::Unreachable
    } else {
        AcknowledgementStoreError::Store(store_error(error))
    }
}

fn corrupt(what: &str) -> AcknowledgementStoreError {
    AcknowledgementStoreError::Store(StoreError::Internal {
        detail: format!("corrupt {what}"),
    })
}

fn column<'r, T>(row: &'r PgRow, name: &str) -> Result<T, AcknowledgementStoreError>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(name).map_err(acknowledgement_error)
}

fn unsigned(value: i64, what: &str) -> Result<u64, AcknowledgementStoreError> {
    u64::try_from(value).map_err(|_| corrupt(what))
}

/// The addresses of `leases` as bound arrays: the run, the path's nodes and indices (each a JSON array, since a path is an
/// array of its own) and the node, in request order.
struct Addresses {
    runs: Vec<Uuid>,
    path_nodes: Vec<String>,
    path_indices: Vec<String>,
    nodes: Vec<String>,
}

impl Addresses {
    fn of<'a>(leases: impl Iterator<Item = &'a LeaseProbe>) -> Self {
        let mut addresses = Self {
            runs: Vec::new(),
            path_nodes: Vec::new(),
            path_indices: Vec::new(),
            nodes: Vec::new(),
        };
        for lease in leases {
            let (nodes, indices) = stored_path_parts(lease.instance.path());
            addresses.runs.push(lease.run_id.as_uuid());
            addresses
                .path_nodes
                .push(serde_json::Value::from(nodes).to_string());
            addresses
                .path_indices
                .push(serde_json::Value::from(indices).to_string());
            addresses
                .nodes
                .push(lease.instance.node().as_str().to_owned());
        }
        addresses
    }
}

/// The request's addresses `u(run_id, path_nodes, path_indices, node, ...)` with their paths decoded once, in a materialized
/// table expression `a`, so that the join to the step row is by its whole primary key (a decoded path in the join itself
/// would be a subquery per row, which no index condition can carry). A statement writes
/// `concat!(decoded_addresses!(), "<the further columns of u>", address_arrays!(), "<their arrays>) AS u(...)) ...")`: both
/// macros are fixed texts, so that the audit of the runtime role's privileges reads the whole statement (a template with an
/// argument in its middle is not expanded by it, and what follows would escape it).
macro_rules! decoded_addresses {
    () => {
        "WITH a AS MATERIALIZED (SELECT u.run_id, \
           ARRAY(SELECT e FROM jsonb_array_elements_text(u.path_nodes::jsonb) WITH ORDINALITY AS x(e, o) ORDER BY o) \
             AS path_nodes, \
           ARRAY(SELECT e::integer FROM jsonb_array_elements_text(u.path_indices::jsonb) WITH ORDINALITY AS x(e, o) \
                 ORDER BY o) AS path_indices, \
           u.node, "
    };
}

/// The arrays of the addresses' first five columns, `$2` to `$6`, for the `unnest` of [`decoded_addresses!`].
macro_rules! address_arrays {
    () => {
        " FROM unnest($2::uuid[], $3::text[], $4::text[], $5::text[], $6::bigint[]"
    };
}

/// The step row `s` of an address of `a`, by its primary key.
macro_rules! addressed_steps {
    () => {
        "s.tenant_id = $1 AND s.run_id = a.run_id AND s.instance_path_nodes = a.path_nodes \
         AND s.instance_path_indices = a.path_indices AND s.node = a.node"
    };
}

/// One acknowledgement's transaction, scoped to the caller's tenant and checked against the call's deadline. It holds its
/// pooled connection itself, not an `sqlx::Transaction`, so that a rollback that does not return within [`ROLLBACK_BOUND`]
/// can discard the connection instead of handing it back (`stream-completion.md`, "The server's acknowledgement", step 1;
/// Task 006-049): closing it makes the server roll the transaction back, and the pool opens another.
pub struct PgAcknowledgementTx {
    /// The connection while its transaction is open; none once it has been committed, rolled back or discarded.
    connection: Option<PoolConnection<Postgres>>,
    tenant: TenantId,
    deadline: CallDeadline,
    /// A statement was cut short on this side: the connection's state is not known, so it is discarded, never rolled back
    /// and handed out again.
    poisoned: bool,
}

impl Drop for PgAcknowledgementTx {
    fn drop(&mut self) {
        // A transaction neither committed nor rolled back (the call's future was dropped mid-way): rolled back off the call,
        // under the same bound, or discarded when there is no runtime to do it on.
        if let Some(connection) = self.connection.take() {
            if self.poisoned {
                discard(connection);
                return;
            }
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    drop(runtime.spawn(roll_back(connection, false, DROPPED_ROLLBACK_BOUND)));
                }
                Err(_) => discard(connection),
            }
        }
    }
}

/// Ends the transaction of `connection` with a `ROLLBACK` that may take `bound`: the connection goes back to the
/// pool when it returns in time, and is discarded otherwise, or when it failed. `stall` makes the `ROLLBACK` never return
/// (the test of the discard).
async fn roll_back(mut connection: PoolConnection<Postgres>, stall: bool, bound: Duration) {
    let ended = {
        let rollback = async {
            if stall {
                std::future::pending::<()>().await;
            }
            sqlx::raw_sql("ROLLBACK").execute(&mut *connection).await
        };
        tokio::time::timeout(bound, rollback).await
    };
    match ended {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "an acknowledgement's rollback failed; its connection is discarded");
            discard(connection);
        }
        Err(_) => {
            metrics::counter!("worker_task_acknowledgement_rollbacks_slow_total").increment(1);
            discard(connection);
        }
    }
}

/// Closes `connection` instead of handing it back to the pool: its state is not known, and the server rolls back whatever
/// transaction it had open when it goes. Counted in `worker_task_acknowledgement_connections_discarded_total`.
fn discard(connection: PoolConnection<Postgres>) {
    metrics::counter!("worker_task_acknowledgement_connections_discarded_total").increment(1);
    drop(connection.detach());
}

impl std::fmt::Debug for PgAcknowledgementTx {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PgAcknowledgementTx")
            .field("tenant", &self.tenant)
            .finish_non_exhaustive()
    }
}

impl PgAcknowledgementTx {
    /// The connection, once the deadline has been checked: past it no statement runs.
    fn connection(&mut self) -> Result<&mut sqlx::PgConnection, AcknowledgementStoreError> {
        if self.deadline.passed() {
            return Err(AcknowledgementStoreError::Deadline);
        }
        self.connection
            .as_deref_mut()
            .ok_or_else(|| corrupt("acknowledgement transaction, already ended"))
    }

    /// How long this side waits for one statement: the call's time left and one statement timeout past it, so that a
    /// statement the server cannot cut short (a network that drops every packet, a flush of the log stalled on its disk)
    /// still ends the call at most one statement timeout past its deadline (`stream-completion.md`, step 1).
    fn statement_bound(&self) -> Duration {
        self.deadline.left(Instant::now()).unwrap_or_default() + ACKNOWLEDGEMENT_STATEMENT_TIMEOUT
    }

    /// The result of a statement waited for under [`Self::statement_bound`]: one cut short poisons the transaction (its
    /// connection is discarded at its end) and is a database not reached.
    fn settle<T>(
        &mut self,
        outcome: Result<Result<T, sqlx::Error>, tokio::time::error::Elapsed>,
    ) -> Result<T, AcknowledgementStoreError> {
        match outcome {
            Ok(result) => result.map_err(acknowledgement_error),
            Err(_) => {
                self.poisoned = true;
                metrics::counter!("worker_task_acknowledgement_statements_cut_total").increment(1);
                Err(AcknowledgementStoreError::Unreachable)
            }
        }
    }

    /// The one settings statement, for the transaction alone: the lock and statement timeouts, and `synchronous_commit`
    /// raised to `local` when it is `off` (never lowering a stronger value), so that a crash the postmaster recovers
    /// without restarting cannot lose a committed acknowledgement (ADR-0099 decision 12).
    async fn settings(&mut self) -> Result<(), AcknowledgementStoreError> {
        let timeout = format!("{}ms", ACKNOWLEDGEMENT_STATEMENT_TIMEOUT.as_millis());
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query(
                "SELECT set_config('lock_timeout', $1, true), set_config('statement_timeout', $1, true), \
                        CASE WHEN current_setting('synchronous_commit') = 'off' \
                             THEN set_config('synchronous_commit', 'local', true) END",
            )
            .bind(timeout)
            .execute(self.connection()?),
        )
        .await;
        self.settle(outcome)?;
        Ok(())
    }

    /// `synchronous_commit` as the acknowledgement's transaction has it, for the test of the durable commit.
    #[doc(hidden)]
    pub async fn synchronous_commit_for_tests(&mut self) -> Result<String, StoreError> {
        let Some(connection) = self.connection.as_deref_mut() else {
            return Err(StoreError::Internal {
                detail: "the acknowledgement's transaction has ended".to_owned(),
            });
        };
        sqlx::query_scalar("SELECT current_setting('synchronous_commit')")
            .fetch_one(connection)
            .await
            .map_err(store_error)
    }

    /// Ends the transaction with a `ROLLBACK` that never returns, for the test that the connection is then discarded (a
    /// test probe: `test_probe_errors` of `scripts/check-architecture.py` refuses a call of it in a production source).
    #[doc(hidden)]
    pub async fn rollback_stalled_for_tests(mut self) {
        if let Some(connection) = self.connection.take() {
            roll_back(connection, true, ROLLBACK_BOUND).await;
        }
    }

    /// The server process of the transaction's connection, for the tests that end it or look for it.
    ///
    /// # Errors
    /// The statement failed, or the transaction has ended.
    #[doc(hidden)]
    pub async fn backend_pid(&mut self) -> Result<i32, StoreError> {
        let Some(connection) = self.connection.as_deref_mut() else {
            return Err(StoreError::Internal {
                detail: "the acknowledgement's transaction has ended".to_owned(),
            });
        };
        sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(connection)
            .await
            .map_err(store_error)
    }
}

/// The 1-based ordinals of `count` addresses in request order.
fn ordinals(count: usize) -> Vec<i64> {
    (1..=count)
        .map(|i| i64::try_from(i).unwrap_or(i64::MAX))
        .collect()
}

fn kind(text: &str) -> Result<TaskKind, AcknowledgementStoreError> {
    match text {
        "FORWARD" => Ok(TaskKind::Forward),
        "COMPENSATION" => Ok(TaskKind::Compensation),
        "RECONCILIATION" => Ok(TaskKind::Reconciliation),
        _ => Err(corrupt("step kind")),
    }
}

fn index(row: &PgRow) -> Result<usize, AcknowledgementStoreError> {
    let ordinal: i64 = column(row, "ordinal")?;
    usize::try_from(ordinal)
        .ok()
        .and_then(|ordinal| ordinal.checked_sub(1))
        .ok_or_else(|| corrupt("request ordinal"))
}

fn locked_lease(row: &PgRow) -> Result<LockedLease, AcknowledgementStoreError> {
    let status: String = column(row, "status")?;
    let attempt: i32 = column(row, "attempt")?;
    let generation: i64 = column(row, "generation")?;
    let hand_back: Option<i64> = column(row, "lease_hand_back_generation")?;
    let offered: Option<i64> = column(row, "lease_offered_generation")?;
    let expires_at: Option<OffsetDateTime> = column(row, "lease_expires_at")?;
    let kind_text: String = column(row, "kind")?;
    let held = matches!(status.as_str(), "LEASED" | "RUNNING");
    Ok(LockedLease {
        index: index(row)?,
        step_id: column(row, "step_id")?,
        leased: status == "LEASED",
        attempt: u32::try_from(attempt).map_err(|_| corrupt("step attempt"))?,
        kind: kind(&kind_text)?,
        token_hash: column(row, "lease_token_hash")?,
        withdrawn_token_hash: column(row, "withdrawn_lease_token_hash")?,
        marks: LeaseMarks {
            deadlines: LeaseDeadlines {
                // a row that holds no lease has no expiry; the decision answers it before it reads the deadlines
                expires_at: expires_at.unwrap_or(OffsetDateTime::UNIX_EPOCH),
                attempt_deadline_at: column(row, "attempt_deadline_at")?,
            },
            held,
            generation: unsigned(generation, "step generation")?,
            hand_back_generation: hand_back
                .map(|g| unsigned(g, "hand-back generation"))
                .transpose()?,
            claimed_at: column(row, "lease_claimed_at")?,
            claimed_on_this_server: column(row, "on_this_server")?,
        },
        offered_generation: offered
            .map(|g| unsigned(g, "offered generation"))
            .transpose()?,
        acknowledged: column(row, "acknowledged")?,
        cancel_requested: column(row, "cancel_requested")?,
        run_stopping: column(row, "run_stopping")?,
        now: column(row, "now")?,
    })
}

impl AcknowledgementTx for PgAcknowledgementTx {
    async fn matched(
        &mut self,
        leases: &[LeaseProbe],
    ) -> Result<Vec<MatchedLease>, AcknowledgementStoreError> {
        let addresses = Addresses::of(leases.iter());
        let hashes: Vec<&[u8]> = leases.iter().map(|l| l.token_hash.as_slice()).collect();
        let tenant = self.tenant;
        let ordinals = ordinals(leases.len());
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query(concat!(
                decoded_addresses!(),
                "u.ordinal, u.hash",
                address_arrays!(),
                ", $7::bytea[]) AS u(run_id, path_nodes, path_indices, node, ordinal, hash)) \
             SELECT a.ordinal, s.namespace_id, s.activity, s.lease_token_hash = a.hash AS current \
               FROM a JOIN steps s ON ",
                addressed_steps!(),
                " WHERE s.lease_token_hash = a.hash OR s.withdrawn_lease_token_hash = a.hash"
            ))
            .bind(tenant.as_uuid())
            .bind(&addresses.runs)
            .bind(&addresses.path_nodes)
            .bind(&addresses.path_indices)
            .bind(&addresses.nodes)
            .bind(&ordinals)
            .bind(&hashes)
            .fetch_all(self.connection()?),
        )
        .await;
        let rows = self.settle(outcome)?;
        rows.iter()
            .map(|row| {
                let current: Option<bool> = column(row, "current")?;
                Ok(MatchedLease {
                    index: index(row)?,
                    namespace: NamespaceId::new(column(row, "namespace_id")?),
                    activity: column(row, "activity")?,
                    current: current.unwrap_or(false),
                })
            })
            .collect()
    }

    async fn bound_patterns(
        &mut self,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<Vec<String>, AcknowledgementStoreError> {
        // The statements of the run transactions' binding check, each checked against the deadline and classified as the
        // acknowledgement classifies (a read-only server or a lost connection is a database not reached).
        let tenant = self.tenant;
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query(crate::bindings::SHARE_ADVISORY_LOCK)
                .bind(crate::bindings::worker_binding_lock_key(
                    tenant, namespace, subject,
                ))
                .execute(self.connection()?),
        )
        .await;
        self.settle(outcome)?;
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query_scalar(
                "SELECT pattern FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .fetch_all(self.connection()?),
        )
        .await;
        self.settle(outcome)
    }

    async fn current_worker_authorized(
        &mut self,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<bool, AcknowledgementStoreError> {
        let tenant = self.tenant;
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query(crate::bindings::SHARE_ADVISORY_LOCK)
                .bind(crate::quotas::secrets_lock_key(tenant, namespace))
                .execute(self.connection()?),
        )
        .await;
        self.settle(outcome)?;
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(
            bound,
            sqlx::query_scalar(crate::execution::LOCK_MEMBERSHIP)
                .bind(tenant.as_uuid())
                .bind(subject)
                .fetch_optional(self.connection()?),
        )
        .await;
        let role: Option<String> = self.settle(outcome)?;
        crate::execution::membership_executes_work(role).map_err(AcknowledgementStoreError::Store)
    }

    async fn lock_matched(
        &mut self,
        leases: &[LeaseProbe],
        indices: &[usize],
    ) -> Result<Vec<LockedLease>, AcknowledgementStoreError> {
        let addresses = Addresses::of(indices.iter().map(|&i| &leases[i]));
        let ordinals: Vec<i64> = indices
            .iter()
            .map(|&i| i64::try_from(i + 1).unwrap_or(i64::MAX))
            .collect();
        let tenant = self.tenant;
        // The rows are locked in primary-key order and read with what the decision needs; the clock is sampled outside the
        // locking query, so once each row's lock is held (a query with a locking clause is never inlined).
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(bound, sqlx::query(concat!(
            decoded_addresses!(),
            "u.ordinal",
            address_arrays!(),
            ") AS u(run_id, path_nodes, path_indices, node, ordinal)), \
             locked AS ( \
               SELECT a.ordinal, s.step_id, s.status, s.attempt, s.generation, s.kind, s.lease_token_hash, \
                      s.withdrawn_lease_token_hash, s.lease_expires_at, s.attempt_deadline_at, \
                      s.lease_hand_back_generation, s.lease_offered_generation, s.lease_claimed_at, ",
            crate::execution::lease_claimed_on_this_server!(),
            " AS on_this_server, s.lease_acknowledged_at IS NOT NULL AS acknowledged, s.cancel_requested, \
                      r.stop_reason IS NOT NULL AS run_stopping \
                 FROM a JOIN steps s ON ",
            addressed_steps!(),
            "    JOIN runs r ON r.tenant_id = s.tenant_id AND r.id = s.run_id \
                ORDER BY s.tenant_id, s.run_id, s.instance_path_nodes, s.instance_path_indices, s.node \
                  FOR NO KEY UPDATE OF s SKIP LOCKED) \
             SELECT locked.*, clock_timestamp() AS now FROM locked"
        ))
        .bind(tenant.as_uuid())
        .bind(&addresses.runs)
        .bind(&addresses.path_nodes)
        .bind(&addresses.path_indices)
        .bind(&addresses.nodes)
        .bind(&ordinals)
        .fetch_all(self.connection()?))
        .await;
        let rows = self.settle(outcome)?;
        rows.iter().map(locked_lease).collect()
    }

    async fn acknowledge(
        &mut self,
        leases: &[LeaseProbe],
        winners: &[(usize, u64)],
    ) -> Result<Vec<WrittenAcknowledgement>, AcknowledgementStoreError> {
        let addresses = Addresses::of(winners.iter().map(|&(i, _)| &leases[i]));
        let ordinals: Vec<i64> = winners
            .iter()
            .map(|&(i, _)| i64::try_from(i + 1).unwrap_or(i64::MAX))
            .collect();
        let generations: Vec<i64> = winners
            .iter()
            .map(|&(_, g)| i64::try_from(g).unwrap_or(i64::MAX))
            .collect();
        let hashes: Vec<&[u8]> = winners
            .iter()
            .map(|&(i, _)| leases[i].token_hash.as_slice())
            .collect();
        let tenant = self.tenant;
        // The rows are held by this transaction; the conditions repeat what the decision found, so a broken caller writes
        // nothing it did not lock and find provisional.
        let bound = self.statement_bound();
        let outcome = tokio::time::timeout(bound, sqlx::query(concat!(
            decoded_addresses!(),
            "u.ordinal, u.generation, u.hash",
            address_arrays!(),
            ", $7::bigint[], $8::bytea[]) AS u(run_id, path_nodes, path_indices, node, ordinal, generation, hash)) ",
            "UPDATE steps s SET lease_acknowledged_at = clock_timestamp() FROM a WHERE ",
            addressed_steps!(),
            " AND s.status = 'LEASED' AND s.generation = a.generation AND s.lease_token_hash = a.hash \
                AND s.lease_offered_generation = s.generation AND s.lease_acknowledged_at IS NULL \
             RETURNING a.ordinal, s.lease_acknowledged_at AS now, s.lease_claimed_at, s.lease_expires_at, \
                       s.attempt_deadline_at"
        ))
        .bind(tenant.as_uuid())
        .bind(&addresses.runs)
        .bind(&addresses.path_nodes)
        .bind(&addresses.path_indices)
        .bind(&addresses.nodes)
        .bind(&ordinals)
        .bind(&generations)
        .bind(&hashes)
        .fetch_all(self.connection()?))
        .await;
        let rows = self.settle(outcome)?;
        rows.iter()
            .map(|row| {
                Ok(WrittenAcknowledgement {
                    index: index(row)?,
                    now: column(row, "now")?,
                    claimed_at: column(row, "lease_claimed_at")?,
                    expires_at: column(row, "lease_expires_at")?,
                    attempt_deadline_at: column(row, "attempt_deadline_at")?,
                })
            })
            .collect()
    }

    async fn commit(mut self) -> Result<(), AcknowledgementStoreError> {
        if self.deadline.passed() {
            self.rollback().await;
            return Err(AcknowledgementStoreError::Deadline);
        }
        let bound = self.statement_bound();
        let Some(mut connection) = self.connection.take() else {
            return Err(corrupt("acknowledgement transaction, already ended"));
        };
        // bounded by the transaction's `statement_timeout` on the server, and by `bound` here: a commit the server does not
        // answer (a network that drops every packet, a flush of the log stalled on its disk) is not known to have committed
        let committed =
            match tokio::time::timeout(bound, sqlx::raw_sql("COMMIT").execute(&mut *connection))
                .await
            {
                Ok(committed) => committed,
                Err(_) => {
                    metrics::counter!("worker_task_acknowledgement_statements_cut_total")
                        .increment(1);
                    discard(connection);
                    return Err(AcknowledgementStoreError::Unreachable);
                }
            };
        match committed {
            Ok(_) => Ok(()),
            // the server answered: the transaction has ended and the connection is idle again
            Err(error @ sqlx::Error::Database(_)) if !store_unreachable(&error) => {
                Err(acknowledgement_error(error))
            }
            // whether it committed is not known on this side; the connection is not handed out again
            Err(error) => {
                discard(connection);
                Err(acknowledgement_error(error))
            }
        }
    }

    async fn rollback(mut self) {
        if let Some(connection) = self.connection.take() {
            if self.poisoned {
                discard(connection);
            } else {
                roll_back(connection, false, ROLLBACK_BOUND).await;
            }
        }
    }
}

impl AcknowledgementStore for PgRunStore {
    type Tx = PgAcknowledgementTx;

    async fn begin_acknowledgement(
        &self,
        tenant: TenantId,
        deadline: CallDeadline,
    ) -> Result<PgAcknowledgementTx, AcknowledgementStoreError> {
        let Some(left) = deadline.left(Instant::now()) else {
            return Err(AcknowledgementStoreError::Deadline);
        };
        // The transaction that did not begin before the deadline, its acquisition included, is a database not reached. The
        // connection is acquired in a task of its own, as `begin_safely` does, which gives up at the deadline too: a call
        // that stops waiting leaves the task to hand the connection it gets back to the pool, never strands it, and no
        // abandoned acquisition waits in the pool's queue past the call.
        let pool = self.pool.clone();
        let acquiring =
            tokio::spawn(async move { tokio::time::timeout(left, pool.acquire()).await });
        let connection = match tokio::time::timeout(left, acquiring).await {
            Ok(Ok(Ok(Ok(connection)))) => connection,
            Ok(Ok(Ok(Err(error)))) => {
                tracing::warn!(%error, "an acknowledgement could not take a connection");
                return Err(AcknowledgementStoreError::Unreachable);
            }
            Ok(Ok(Err(_)) | Err(_)) | Err(_) => return Err(AcknowledgementStoreError::Unreachable),
        };
        // The guard holds the connection before `BEGIN` is sent: a call dropped while it waits for the answer leaves the
        // connection to the guard's `Drop`, which rolls back (or discards) it, never to the pool inside a transaction.
        let mut tx = PgAcknowledgementTx {
            connection: Some(connection),
            tenant,
            deadline,
            poisoned: false,
        };
        let Some(left) = deadline.left(Instant::now()) else {
            tx.poisoned = true;
            return Err(AcknowledgementStoreError::Unreachable);
        };
        let begun =
            tokio::time::timeout(left, sqlx::raw_sql("BEGIN").execute(tx.connection()?)).await;
        match begun {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "an acknowledgement could not begin its transaction");
                tx.poisoned = true;
                return Err(AcknowledgementStoreError::Unreachable);
            }
            // a `BEGIN` cut short leaves the connection in a state not known
            Err(_) => {
                tx.poisoned = true;
                return Err(AcknowledgementStoreError::Unreachable);
            }
        }
        if let Err(error) = tx.settings().await {
            tx.rollback().await;
            return Err(error);
        }
        Ok(tx)
    }

    async fn run_of_another_tenant(
        &self,
        tenant: TenantId,
        runs: &[RunId],
    ) -> Result<Option<RunId>, StoreError> {
        let ids: Vec<Uuid> = runs.iter().map(|run| run.as_uuid()).collect();
        // Bounded: with no index on the run id alone the check can scan the runs (D006-110). It never waits for a connection
        // the answer path needs, and the runs of the caller's own tenant are taken out first, by the primary key, in a table
        // expression of their own, so that only the runs of no row of the tenant reach the scan.
        let Some(mut connection) = self.pool.try_acquire() else {
            return Err(StoreError::DatabaseLimit);
        };
        let mut tx = sqlx::Connection::begin(&mut *connection)
            .await
            .map_err(store_error)?;
        sqlx::query("SELECT set_config('statement_timeout', '1000ms', true)")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let found: Option<Uuid> = sqlx::query_scalar(
            "WITH foreign_or_unknown AS MATERIALIZED ( \
               SELECT u.id FROM unnest($1::uuid[]) AS u(id) \
                WHERE NOT EXISTS (SELECT 1 FROM runs WHERE tenant_id = $2 AND id = u.id)) \
             SELECT f.id FROM foreign_or_unknown f \
              WHERE EXISTS (SELECT 1 FROM runs WHERE id = f.id AND tenant_id <> $2) \
              LIMIT 1",
        )
        .bind(&ids)
        .bind(tenant.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.rollback().await.map_err(store_error)?;
        Ok(found.map(RunId::new))
    }

    async fn record_cross_tenant(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        sagat_application::runs::RunStore::record_denial(self, audit).await
    }
}

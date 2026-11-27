//! Design 041, task T041-6: what the step-log suites share.
//!
//! * A *session fixture*: a worker registered the real way at the world's minor, its `worker_sessions` row raised to
//!   protocol minor 7 (the negotiation of 1.7 through the RPC is tested by the `rpc` module), and an attempt of a run
//!   leased to it.
//! * A *wrapper* of the store's transaction, [`Hooked`], with six hooks: fail before the commit; commit and then fail
//!   as a lost answer would; hold after T2 (no lock held); hold after T3 (the policy fences and the attempt's stream
//!   lock held, L1 and L1b); hold after T7 (the stream row held since T4, L2); hold after T8 (the writer's own quota
//!   lease held, L3b). A hold after a T8 that refilled (the usage row held, L4) is the hold after T8 once the fixture
//!   has emptied the writer's quota leases ([`empty_quota_leases`]).
//!
//! The ingest runs on the world's execution pool, the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE`; the fixtures
//! are the owner's.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::error::StoreError;
use sagat_application::step_logs::{
    AttemptRef, BatchAnswer, CounterUpdate, InsertOutcome, InsertRows, LeaseRead, SessionRef,
    StepLogBatch, StepLogIngest, StepLogPolicy, StepLogRecordIn, StepLogStore, StepLogTx,
    StreamRead, StreamRef,
};
use sagat_application::worker::ClaimedTask;
use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Principal, Role, TenantId};
use sagat_domain::step_logs::{StepLogLevel, TenantAllowance};
use sagat_infra_postgres::{PgStepLogStore, PgStepLogTx};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Notify;
use uuid::Uuid;

use super::engine::{Session, World, activity, document};

/// The lock timeout of a store whose batch a test makes wait on a held lock for longer than 200 ms.
pub const HELD_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// An attempt leased to a worker whose session was accepted at 1.7.
pub struct Attempt {
    pub session: Session,
    pub task: ClaimedTask,
    pub run: RunId,
}

/// A workflow of one activity `act.<node>` published in the world's namespace.
pub async fn one_activity(w: &World, node: &str) -> sagat_domain::definition::WorkflowId {
    w.publish(document(
        &format!("logs-{node}"),
        node,
        vec![activity(node)],
    ))
    .await
    .0
}

/// Raises a session to protocol minor 7, as the registration of a 1.7 worker does.
pub async fn accept_at_1_7(w: &World, session: &Session) {
    sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 7 WHERE id = $1")
        .bind(session.worker_id)
        .execute(&w.db.pool)
        .await
        .expect("raise the session to 1.7");
}

/// A run of a one-activity workflow whose step is leased to a fresh 1.7 session of the world's worker.
pub async fn leased(w: &World, node: &str) -> Attempt {
    let workflow = one_activity(w, node).await;
    let session = w.session(&[&format!("act.{node}")]).await;
    accept_at_1_7(w, &session).await;
    let run = w.start(workflow, &json!({})).await;
    let task = w.one_task(&session).await;
    Attempt { session, task, run }
}

/// A record at `sequence` whose message is `message`, level info.
pub fn record(sequence: u64, message: &str) -> StepLogRecordIn {
    StepLogRecordIn {
        sequence,
        level: Some(StepLogLevel::Info),
        worker_time: None,
        message: message.to_owned(),
        exception: String::new(),
        fields: Vec::new(),
        truncated: false,
    }
}

/// Records at each of `sequences`, each message `line <sequence>`.
pub fn records(sequences: impl IntoIterator<Item = u64>) -> Vec<StepLogRecordIn> {
    sequences
        .into_iter()
        .map(|sequence| record(sequence, &format!("line {sequence}")))
        .collect()
}

/// The batch of `records` for `attempt`, with an encoded size well under the bound.
pub fn batch(attempt: &Attempt, records: Vec<StepLogRecordIn>) -> StepLogBatch {
    StepLogBatch {
        worker_id: attempt.session.worker_id,
        session_token: attempt.session.token.clone(),
        run_id: attempt.task.run_id,
        step_id: attempt.task.step_id,
        instance: attempt.task.instance.clone(),
        fence: attempt.task.fence,
        lease_token: attempt.task.lease_token.expose().to_owned(),
        encoded_len: 1_024,
        records,
    }
}

/// The ingest over the world's execution pool, capture on.
pub fn ingest(w: &World) -> StepLogIngest<PgStepLogStore, SystemClock> {
    StepLogIngest::new(
        PgStepLogStore::new(w.execution_pool.clone()),
        SystemClock,
        true,
    )
}

/// The ingest over a pool of its own of `connections` (the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE`): a writer
/// whose quota lease is its own (a quota lease belongs to the server backend that charges, D041-117).
pub async fn ingest_on_own_pool(
    w: &World,
    connections: u32,
) -> StepLogIngest<PgStepLogStore, SystemClock> {
    StepLogIngest::new(
        PgStepLogStore::new(pool_of(w, connections).await),
        SystemClock,
        true,
    )
}

/// A pool of `connections` of the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE`, else of the owner.
pub async fn pool_of(w: &World, connections: u32) -> PgPool {
    if super::as_runtime_role() {
        w.db.runtime_pool_with(connections).await
    } else {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(connections)
            .connect(&w.db.url)
            .await
            .expect("a pool of the owner")
    }
}

/// Sends `records` for `attempt` through `ingest` and returns the answer, failing on an error.
pub async fn send(
    ingest: &StepLogIngest<impl StepLogStore, SystemClock>,
    attempt: &Attempt,
    records: Vec<StepLogRecordIn>,
) -> BatchAnswer {
    ingest
        .append(&attempt.session.principal, &batch(attempt, records))
        .await
        .expect("the batch is answered")
}

/// What the stream of an attempt holds, read by SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamRow {
    pub records: i32,
    pub bytes: i64,
    pub last_sequence: i64,
    pub ceiling_sequence: Option<i64>,
    pub ceiling_reason: Option<String>,
    pub dropped_by_worker: i64,
    pub dropped_by_server: i64,
    pub truncated_records: i32,
}

/// The stream row of `attempt`, if one exists.
pub async fn stream_of(w: &World, attempt: &Attempt) -> Option<StreamRow> {
    use sqlx::Row as _;
    let row = sqlx::query(
        "SELECT records, bytes, last_sequence, ceiling_sequence, ceiling_reason, dropped_by_worker, \
                dropped_by_server, truncated_records \
           FROM step_log_streams WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3 AND attempt = $4",
    )
    .bind(attempt.session.principal.tenant_id().as_uuid())
    .bind(attempt.run.as_uuid())
    .bind(attempt.task.step_id)
    .bind(i32::try_from(attempt.task.fence.attempt).expect("an attempt"))
    .fetch_optional(&w.db.pool)
    .await
    .expect("the stream")?;
    let get = |name: &str| -> i64 { row.try_get(name).expect(name) };
    let get32 = |name: &str| -> i32 { row.try_get(name).expect(name) };
    Some(StreamRow {
        records: get32("records"),
        bytes: get("bytes"),
        last_sequence: get("last_sequence"),
        ceiling_sequence: row.try_get("ceiling_sequence").expect("ceiling_sequence"),
        ceiling_reason: row.try_get("ceiling_reason").expect("ceiling_reason"),
        dropped_by_worker: get("dropped_by_worker"),
        dropped_by_server: get("dropped_by_server"),
        truncated_records: get32("truncated_records"),
    })
}

/// The lines stored for `attempt`: (sequence, kind, message), in sequence order.
pub async fn lines_of(w: &World, attempt: &Attempt) -> Vec<(i64, String, Option<String>)> {
    sqlx::query_as(
        "SELECT l.sequence, l.kind, l.message FROM step_log_lines l \
           JOIN step_log_streams s ON s.tenant_id = l.tenant_id AND s.stream_id = l.stream_id \
          WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.step_id = $3 ORDER BY l.sequence",
    )
    .bind(attempt.session.principal.tenant_id().as_uuid())
    .bind(attempt.run.as_uuid())
    .bind(attempt.task.step_id)
    .fetch_all(&w.db.pool)
    .await
    .expect("the lines")
}

/// U = S + Σ used of the world's first tenant.
pub async fn step_log_usage(w: &World) -> i64 {
    sqlx::query_scalar("SELECT sagat_quota_step_log_usage($1)")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the usage")
}

/// Sets the first tenant's `tenant_step_log_bytes` and its ceiling to `bytes` by the owner, as an operator's writes;
/// each fires the reclaim of every quota lease of the tenant (ADR-0097 decision 6).
pub async fn set_tenant_limit(w: &World, bytes: i64) {
    let tenant = w.admin.tenant_id().as_uuid();
    let mut tx = w.db.pool.begin().await.expect("a transaction");
    for statement in [
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'tenant_step_log_bytes', $2, 'test') \
         ON CONFLICT (tenant_id, resource) DO UPDATE SET max_value = EXCLUDED.max_value",
        "DELETE FROM quota_limits WHERE tenant_id = $1 AND namespace_id IS NULL \
           AND resource = 'tenant_step_log_bytes' AND $2::bigint IS NOT NULL",
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_step_log_bytes', $2, 'test')",
    ] {
        sqlx::query(statement)
            .bind(tenant)
            .bind(bytes)
            .execute(&mut *tx)
            .await
            .expect("the tenant's step-log limit");
    }
    tx.commit().await.expect("the limit is set");
}

/// Empties every quota lease of the first tenant: a change of its `tenant_step_log_bytes` and its return to the value
/// it had, each of which reclaims every quota lease (ADR-0097 decision 6), so that the next charge refills.
pub async fn empty_quota_leases(w: &World) {
    let limit: i64 = sqlx::query_scalar("SELECT max_value FROM sagat_quota_step_log_allowance($1)")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the limit");
    set_tenant_limit(w, limit + 1).await;
    set_tenant_limit(w, limit).await;
    let granted: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(granted - used), 0)::bigint FROM step_log_quota_leases WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the leases");
    assert_eq!(granted, 0, "no quota lease of the tenant keeps room");
}

/// Sets a namespace resource of the world's namespace by the quota service, as an administrator would.
pub async fn set_namespace_policy(
    w: &World,
    resource: sagat_domain::quota::QuotaResource,
    value: u64,
) {
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            resource,
            value,
            &super::engine::ctx(),
        )
        .await
        .expect("set the namespace's step-log resource");
}

/// A member of the world's first tenant with `role`, bound to every activity, with a 1.7 session for `activity`.
pub async fn member_session(w: &World, name: &str, role: Role, activity: &str) -> Session {
    let principal: Principal = w.member(name, role).await;
    let session = w.session_as(&principal, &[activity], 10).await;
    accept_at_1_7(w, &session).await;
    session
}

/// Backends of this database waiting on a lock, other than the asking one.
pub async fn waiting_on_locks(pool: &PgPool) -> Vec<(i32, String)> {
    sqlx::query_as(
        "SELECT pid, coalesce(left(query, 120), '') FROM pg_stat_activity \
          WHERE datname = current_database() AND wait_event_type = 'Lock' AND pid <> pg_backend_pid()",
    )
    .fetch_all(pool)
    .await
    .expect("pg_stat_activity")
}

/// Waits until a backend of this database waits on a lock held by `holder`.
pub async fn until_blocked_by(pool: &PgPool, holder: i32) -> i32 {
    super::eventually_within(
        "a backend waits on the held lock",
        Duration::from_secs(30),
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
              WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
            )
            .bind(holder)
            .fetch_optional(pool)
            .await
            .expect("pg_stat_activity")
        },
    )
    .await
}

// ---------------------------------------------------------------------------------- the wrapper

/// Where the wrapper stops or fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    /// The commit is not run: the transaction is rolled back and the batch fails.
    FailBeforeCommit,
    /// The commit runs and the batch then fails, as a lost answer would.
    CommitThenFail,
    /// Held after T2: no lock held.
    HoldAfterLease,
    /// Held after T3: the policy fences and the attempt's stream lock held (L1, L1b).
    HoldAfterPolicies,
    /// Held after T7: the stream row held since T4 (L2).
    HoldAfterAuthority,
    /// Held after T8: the writer's own quota lease held (L3b), and the usage row too when T8 refilled (L4).
    HoldAfterInsert,
}

/// The meeting point of a held batch and its test: the batch says it reached its hook and which backend holds its
/// transaction, then waits to be released.
#[derive(Debug, Default)]
pub struct Gate {
    reached: Notify,
    release: Notify,
    pid: std::sync::Mutex<Option<i32>>,
}

impl Gate {
    /// Waits until the batch reached its hook and returns the backend that holds its transaction.
    pub async fn reached(&self) -> i32 {
        super::bounded::within(
            "the batch reaches its hook",
            Duration::from_secs(30),
            self.reached.notified(),
        )
        .await;
        self.pid
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .expect("the backend of the held batch")
    }

    /// Lets the held batch go on.
    pub fn release(&self) {
        self.release.notify_one();
    }
}

/// The store, with a hook on its transactions.
#[derive(Debug, Clone)]
pub struct Hooked {
    pub inner: PgStepLogStore,
    pub hook: Hook,
    pub gate: Arc<Gate>,
}

impl Hooked {
    /// The store over `pool` with `hook`, whose lock timeout lets a batch wait on a held lock.
    pub fn new(pool: PgPool, hook: Hook) -> Self {
        Self {
            inner: PgStepLogStore::new(pool).with_lock_timeout(HELD_LOCK_TIMEOUT),
            hook,
            gate: Arc::new(Gate::default()),
        }
    }
}

/// The transaction of a [`Hooked`] store.
#[derive(Debug)]
pub struct HookedTx {
    inner: PgStepLogTx,
    hook: Hook,
    gate: Arc<Gate>,
}

impl HookedTx {
    async fn hold(&mut self, at: Hook) {
        if self.hook != at {
            return;
        }
        let pid = self.inner.backend_pid().await.expect("the backend");
        *self
            .gate
            .pid
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);
        self.gate.reached.notify_one();
        self.gate.release.notified().await;
    }
}

fn lost() -> StoreError {
    StoreError::Unavailable {
        detail: "sqlstate:none".into(),
    }
}

impl StepLogStore for Hooked {
    type Tx = HookedTx;

    async fn begin(&self) -> Result<HookedTx, StoreError> {
        Ok(HookedTx {
            inner: self.inner.begin().await?,
            hook: self.hook,
            gate: Arc::clone(&self.gate),
        })
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        self.inner.run_exists_outside(tenant, run).await
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.inner.record_denial(audit).await
    }
}

impl StepLogTx for HookedTx {
    async fn assign_transaction_id(&mut self) -> Result<(), StoreError> {
        self.inner.assign_transaction_id().await
    }

    async fn lease(&mut self, attempt: &AttemptRef<'_>) -> Result<Option<LeaseRead>, StoreError> {
        let lease = self.inner.lease(attempt).await?;
        self.hold(Hook::HoldAfterLease).await;
        Ok(lease)
    }

    async fn session(&mut self, session: &SessionRef<'_>) -> Result<bool, StoreError> {
        self.inner.session(session).await
    }

    async fn policies(&mut self, stream: &StreamRef<'_>) -> Result<StepLogPolicy, StoreError> {
        let policy = self.inner.policies(stream).await?;
        self.hold(Hook::HoldAfterPolicies).await;
        Ok(policy)
    }

    async fn stream(&mut self, stream: &StreamRef<'_>) -> Result<StreamRead, StoreError> {
        self.inner.stream(stream).await
    }

    async fn tenant_allowance(&mut self, tenant: TenantId) -> Result<TenantAllowance, StoreError> {
        self.inner.tenant_allowance(tenant).await
    }

    async fn binding_patterns(
        &mut self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<Vec<String>, StoreError> {
        self.inner
            .binding_patterns(tenant, namespace, subject)
            .await
    }

    async fn membership_role(
        &mut self,
        tenant: TenantId,
        subject: &str,
    ) -> Result<Option<Role>, StoreError> {
        let role = self.inner.membership_role(tenant, subject).await?;
        self.hold(Hook::HoldAfterAuthority).await;
        Ok(role)
    }

    async fn insert(&mut self, rows: &InsertRows<'_>) -> Result<InsertOutcome, StoreError> {
        let outcome = self.inner.insert(rows).await?;
        self.hold(Hook::HoldAfterInsert).await;
        Ok(outcome)
    }

    async fn record_counters(&mut self, update: &CounterUpdate) -> Result<(), StoreError> {
        self.inner.record_counters(update).await
    }

    async fn commit(self) -> Result<(), StoreError> {
        match self.hook {
            Hook::FailBeforeCommit => {
                self.inner.rollback().await?;
                Err(lost())
            }
            Hook::CommitThenFail => {
                self.inner.commit().await?;
                Err(lost())
            }
            _ => self.inner.commit().await,
        }
    }

    async fn rollback(self) -> Result<(), StoreError> {
        self.inner.rollback().await
    }
}

/// An ingest over a [`Hooked`] store on the world's execution pool, and the gate of its held batch.
pub fn hooked(w: &World, hook: Hook) -> (Arc<StepLogIngest<Hooked, SystemClock>>, Arc<Gate>) {
    hooked_on(w.execution_pool.clone(), hook)
}

/// An ingest over a [`Hooked`] store on `pool`, and the gate of its held batch.
pub fn hooked_on(pool: PgPool, hook: Hook) -> (Arc<StepLogIngest<Hooked, SystemClock>>, Arc<Gate>) {
    let store = Hooked::new(pool, hook);
    let gate = Arc::clone(&store.gate);
    (Arc::new(StepLogIngest::new(store, SystemClock, true)), gate)
}

/// Starts `records` of `attempt` through `ingest` in a task of its own.
pub fn spawn_send(
    ingest: &Arc<StepLogIngest<Hooked, SystemClock>>,
    attempt: &Attempt,
    records: Vec<StepLogRecordIn>,
) -> tokio::task::JoinHandle<Result<BatchAnswer, sagat_application::error::AppError>> {
    let ingest = Arc::clone(ingest);
    let principal = attempt.session.principal.clone();
    let batch = batch(attempt, records);
    tokio::spawn(async move { ingest.append(&principal, &batch).await })
}

/// A fresh stream key of no attempt, for a scratch test.
pub fn nobody() -> Uuid {
    Uuid::now_v7()
}

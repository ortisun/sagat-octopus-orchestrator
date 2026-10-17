//! A declared-state run inside the engine world, and the harness the operation fault tests share.
//!
//! The world has the real worker protocol, scheduler and secret store over PostgreSQL, so an Update
//! or a Query can race an activity completion or a timer in the same database. Every race is
//! ordered by the database's own lock queue, never by a sleep: the test holds the run row, starts
//! the contenders one after the other, and waits until PostgreSQL reports each of them blocked on a
//! lock before it lets go.

use std::future::Future;

use sagat_application::declared_operations::{
    DeclaredOperationEvaluator, DeclaredOperationStore, DeclaredOperationStoreError, DeclaredQuery,
    DeclaredUpdate,
};
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{
    OperationAddress, OperationError, OperationScope, RunId, UpdateKey, UpdateReceiptStatus,
    UpdateRequest, UpdateResolution,
};
use sagat_domain::identity::{NamespaceId, Principal};
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_ir::JsonDeclaredOperationEvaluator;
use sagat_infra_postgres::PgDeclaredOperationStore;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::support::engine::{World, document, world};

pub(super) const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";

/// The declared state, one Query reading it and two Updates writing it. `balance.observe` returns
/// what the run had completed for the node `work` when it ran, so the result tells which side of a
/// race the Update landed on.
pub(super) fn state_block() -> Value {
    let new_state = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"balance": {"type": "integer", "minimum": 0}},
        "required": ["balance"]
    });
    let arguments = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"allowed": {"type": "boolean"}, "nextState": new_state},
        "required": ["allowed", "nextState"]
    });
    json!({
        "schema": new_state,
        "initial": "input.account",
        "queries": {
            "balance.read": {
                "argsSchema": {"type": "integer"},
                "resultSchema": {"type": "integer"},
                "expression": "state.balance + args",
                "disclosure": "declared-business-result-v1"
            },
            "work.read": {
                "argsSchema": {"type": "integer"},
                "resultSchema": {"type": "integer"},
                "expression": "has(completedResults.work) ? completedResults.work.amount + args : args",
                "disclosure": "declared-business-result-v1"
            }
        },
        "updates": {
            "balance.replace": {
                "argsSchema": arguments,
                "validate": "args.allowed",
                "state": "args.nextState",
                "result": "newState.balance",
                "resultSchema": {"type": "integer", "minimum": 0},
                "disclosure": "declared-business-result-v1"
            },
            "balance.observe": {
                "argsSchema": arguments,
                "validate": "args.allowed",
                "state": "args.nextState",
                "result": "has(completedResults.work) ? completedResults.work.amount : 0",
                "resultSchema": {"type": "integer", "minimum": 0},
                "disclosure": "declared-business-result-v1"
            }
        }
    })
}

/// A workflow document with the declared state block attached to `nodes`.
pub(super) fn declared(name: &str, entrypoint: &str, nodes: Vec<Value>) -> Value {
    let mut document = document(name, entrypoint, nodes);
    document["state"] = state_block();
    document
}

pub(super) fn input(balance: u64) -> Value {
    json!({"account": {"balance": balance}, "key": "account-7"})
}

/// A published declared workflow, one started run, and the store that serves its operations.
pub(super) struct Declared {
    pub(super) w: World,
    pub(super) workflow: WorkflowId,
    pub(super) run: RunId,
    pub(super) ops: Ops,
}

pub(super) async fn start(document: Value, input: &Value) -> Declared {
    let w = world().await;
    let (workflow, _) = w.publish(document).await;
    let run = w.start(workflow, input).await;
    let ops = Ops::on(&w, w.db.pool.clone());
    Declared {
        w,
        workflow,
        run,
        ops,
    }
}

/// What an applied Update answered: the durable resolution and the retained result.
#[derive(Debug)]
pub(super) struct Answer {
    pub(super) resolution: UpdateResolution,
    pub(super) result: Option<Value>,
}

impl Answer {
    pub(super) fn status(&self) -> UpdateReceiptStatus {
        self.resolution.receipt().status()
    }
}

/// What a Query answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Read {
    pub(super) run_version: u64,
    pub(super) state_revision: u64,
    pub(super) result: Value,
}

/// The operation store of one run, driven the way the HTTP routes drive it.
#[derive(Clone)]
pub(super) struct Ops {
    store: PgDeclaredOperationStore,
    actor: Principal,
    namespace: NamespaceId,
}

impl Ops {
    pub(super) fn on(w: &World, pool: PgPool) -> Self {
        Self {
            store: PgDeclaredOperationStore::new(pool),
            actor: w.admin.clone(),
            namespace: w.namespace.id(),
        }
    }

    /// The same operations served over another pool.
    pub(super) fn over(&self, pool: PgPool) -> Self {
        Self {
            store: PgDeclaredOperationStore::new(pool),
            ..self.clone()
        }
    }

    fn scope(&self, run: RunId) -> OperationScope {
        OperationScope::new(
            self.actor.tenant_id(),
            self.namespace,
            OperationAddress::Run(run),
        )
    }

    fn audit(&self, action: AuditAction, run: RunId) -> AuditRecord {
        AuditRecord::by(
            &self.actor,
            action,
            ResourceType::Run,
            run.to_string(),
            Some(TRACE.to_owned()),
            crate::support::now(),
        )
    }

    pub(super) fn update_command(
        &self,
        run: RunId,
        operation: &str,
        key: &str,
        args: &Value,
        expected_revision: Option<u64>,
    ) -> DeclaredUpdate {
        let arguments =
            DeclaredOperationArguments::parse(&serde_json::to_vec(args).expect("arguments"))
                .expect("arguments within bounds");
        let now = crate::support::now();
        DeclaredUpdate::new(
            UpdateRequest::new(
                self.scope(run),
                run,
                ActivityName::parse(operation).expect("operation"),
                UpdateKey::parse(key).expect("update key"),
                Digest::new([0; 32]),
                expected_revision,
                self.actor.subject().clone(),
            ),
            arguments,
            self.audit(AuditAction::RunUpdated, run),
            Uuid::now_v7(),
            now,
        )
    }

    /// An Update exactly as the HTTP route serves it: the serialized decision, then the retained
    /// result of an accepted one.
    pub(super) async fn update(
        &self,
        run: RunId,
        operation: &str,
        key: &str,
        args: &Value,
        expected_revision: Option<u64>,
    ) -> Result<Answer, DeclaredOperationStoreError> {
        let command = self.update_command(run, operation, key, args, expected_revision);
        self.apply(&command).await
    }

    pub(super) async fn apply(
        &self,
        command: &DeclaredUpdate,
    ) -> Result<Answer, DeclaredOperationStoreError> {
        self.apply_gated(command, || {}).await
    }

    /// [`Self::apply`] that calls `gate` inside the serialized section: after the run row is locked
    /// and the snapshot read, before anything is written. The gate blocks its thread, so the test
    /// that uses it runs on a multi-threaded runtime.
    pub(super) async fn apply_gated(
        &self,
        command: &DeclaredUpdate,
        gate: impl FnOnce() + Send,
    ) -> Result<Answer, DeclaredOperationStoreError> {
        let evaluator = JsonDeclaredOperationEvaluator;
        let operation = command.request().operation().clone();
        let evaluated_arguments = command.arguments().clone();
        let resolution = self
            .store
            .apply_update(
                command,
                |snapshot, arguments| {
                    DeclaredOperationEvaluator::validate_update_arguments(
                        &evaluator, snapshot, &operation, arguments,
                    )
                    .map_err(|_| OperationError::InvalidArguments)
                },
                |snapshot| {
                    gate();
                    DeclaredOperationEvaluator::update(
                        &evaluator,
                        snapshot,
                        &operation,
                        &evaluated_arguments,
                    )
                    .map_err(|_| OperationError::EvaluationFailed)
                },
            )
            .await?;
        if resolution.receipt().status() == UpdateReceiptStatus::Rejected {
            return Ok(Answer {
                resolution,
                result: None,
            });
        }
        let result = self
            .store
            .update_result(command, &resolution)
            .await?
            .map(|write| serde_json::from_slice(write.content()).expect("result JSON"));
        Ok(Answer { resolution, result })
    }

    /// A Query exactly as the HTTP route serves it.
    pub(super) async fn query(
        &self,
        run: RunId,
        operation: &str,
        args: &Value,
    ) -> Result<Read, DeclaredOperationStoreError> {
        let evaluator = JsonDeclaredOperationEvaluator;
        let operation = ActivityName::parse(operation).expect("operation");
        let query = DeclaredQuery::new(
            self.scope(run),
            run,
            operation.clone(),
            self.actor.subject().clone(),
        );
        let snapshot = self
            .store
            .query_snapshot(&query, &self.audit(AuditAction::RunQueried, run))
            .await?;
        let arguments =
            DeclaredOperationArguments::parse(&serde_json::to_vec(args).expect("arguments"))
                .expect("arguments within bounds");
        let evaluated =
            DeclaredOperationEvaluator::query(&evaluator, &snapshot, &operation, &arguments)
                .expect("the declared query evaluates");
        Ok(Read {
            run_version: evaluated.response().run_version(),
            state_revision: evaluated.response().state_revision(),
            result: serde_json::from_slice(evaluated.result().content()).expect("result JSON"),
        })
    }
}

/// The balance argument of an Update replacing the state.
pub(super) fn replace(balance: u64) -> Value {
    json!({"allowed": true, "nextState": {"balance": balance}})
}

// ------------------------------------------------------------------------------ lock queue

/// A transaction that holds the row lock of one run until it is released.
pub(super) struct RunHold {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl RunHold {
    pub(super) async fn take(pool: &PgPool, w: &World, run: RunId) -> Self {
        let mut tx = pool.begin().await.expect("holding transaction");
        sqlx::query("SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("hold the run row");
        Self { tx }
    }

    /// Lets go without changing anything.
    pub(super) async fn release(self) {
        self.tx.rollback().await.expect("release the run row");
    }
}

/// Completes when at least `count` backends of this database wait on a lock. This watches an
/// observable condition of the database; it is not a pause.
pub(super) async fn blocked(pool: &PgPool, count: i64) {
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting >= count {
            return;
        }
        tokio::task::yield_now().await;
    }
}

/// Starts `first`, then `second`, while the run row is held; each is proven to be waiting on the
/// database before the next starts or the row is released, so `first` is queued ahead of `second`
/// for the lock. Returns what each produced.
pub(super) async fn race<A, B>(
    w: &World,
    run: RunId,
    first: impl Future<Output = A>,
    second: impl Future<Output = B>,
) -> (A, B) {
    let mut first = Box::pin(first);
    let mut second = Box::pin(second);
    let hold = RunHold::take(&w.db.pool, w, run).await;
    tokio::select! {
        _ = &mut first => panic!("the first contender finished while the run row was held"),
        () = blocked(&w.db.pool, 1) => {}
    }
    tokio::select! {
        _ = &mut first => panic!("the first contender finished while the run row was held"),
        _ = &mut second => panic!("the second contender finished while the run row was held"),
        () = blocked(&w.db.pool, 2) => {}
    }
    hold.release().await;
    tokio::join!(first, second)
}

// ------------------------------------------------------------------------------ evidence

/// The committed journal of the run in order: `(seq, event type)`.
pub(super) async fn journal(w: &World, run: RunId) -> Vec<(i64, String)> {
    sqlx::query_as(
        "SELECT seq, event_type FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("journal")
}

/// Position of the first event of `kind` in a journal.
pub(super) fn position(journal: &[(i64, String)], kind: &str) -> usize {
    journal
        .iter()
        .position(|(_, event)| event == kind)
        .unwrap_or_else(|| panic!("no {kind} event in {journal:?}"))
}

/// `(state revision, receipts, state.updated events, run.state_updated outbox rows)`.
pub(super) async fn update_evidence(w: &World, run: RunId) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x \
                  WHERE x.tenant_id=s.tenant_id AND x.resolved_run_id=s.run_id), \
                (SELECT count(*) FROM run_events e \
                  WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
                    AND e.event_type='state.updated'), \
                (SELECT count(*) FROM event_outbox o \
                  WHERE o.tenant_id=s.tenant_id AND o.aggregate_id=s.run_id \
                    AND o.event_type='run.state_updated') \
         FROM workflow_operation_states s WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("update evidence")
}

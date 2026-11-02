//! Fault injection at task-context materialization; storage, reducer and worker are real.
use super::*;
use sagat_application::{
    execution::{ExpressionEvaluator, ExpressionFailure, ReconcileAnswer, RunData},
    loop_progress::{LoopCheckpointEvaluation, LoopEntryEvaluation},
    worker::MAX_TASK_INPUT_BYTES,
};
use sagat_domain::{
    execution::{CorrelationKey, ExecutionInstanceId},
    workflow::{Expression, NodeId},
};

#[derive(Clone, Copy)]
struct OversizedTaskContext;

// Delegate expression semantics; inject only a legal JSON context exceeding the wire ceiling.
// This tests the admission boundary, not how JSON numbers expand (covered in hardening.rs).
macro_rules! delegate {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $result:ty;)+) => {
        $(fn $name(&self, $($arg: $ty),*) -> $result {
            JsonEvaluator.$name($($arg),*)
        })+
    };
}

impl ExpressionEvaluator for OversizedTaskContext {
    delegate! {
        fn condition(expression: &Expression, data: &RunData) -> Result<bool, ExpressionFailure>;
        fn condition_instance(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<bool, ExpressionFailure>;
        fn correlation_key(expression: &Expression, data: &RunData) -> Result<CorrelationKey, ExpressionFailure>;
        fn correlation_key_instance(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<CorrelationKey, ExpressionFailure>;
        fn idempotency_key(template: &Expression, data: &RunData, node: &NodeId) -> Result<String, ExpressionFailure>;
        fn idempotency_key_instance(template: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<String, ExpressionFailure>;
        fn child_input(expression: Option<&Expression>, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn run_result(expression: &Expression, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn continuation_value(expression: &Expression, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn collection_items(expression: &Expression, data: &RunData) -> Result<Vec<Vec<u8>>, ExpressionFailure>;
        fn collection_return(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<Vec<u8>, ExpressionFailure>;
        fn collection_member_result(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId, body_result: &[u8]) -> Result<Vec<u8>, ExpressionFailure>;
        fn loop_entry(initial: &Expression, condition: Option<&Expression>, data: &RunData) -> Result<LoopEntryEvaluation, ExpressionFailure>;
        fn loop_checkpoint(result: &Expression, carry: &Expression, condition: &Expression, data: &RunData, return_instance: &ExecutionInstanceId) -> Result<LoopCheckpointEvaluation, ExpressionFailure>;
        fn reconcile_answer(output: &[u8]) -> Result<ReconcileAnswer, ExpressionFailure>;
        fn event_document(event: &sagat_application::dispatch::EventView<'_>) -> Vec<u8>;
        fn event_filter(filter: &Expression, document: &[u8]) -> Result<bool, ExpressionFailure>;
        fn is_json(bytes: &[u8]) -> bool;
        fn is_json_object(bytes: &[u8]) -> bool;
    }

    fn task_input(&self, _data: &RunData, _node: &NodeId) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"large":"x".repeat(MAX_TASK_INPUT_BYTES)}))
            .expect("legal oversized JSON context")
    }

    fn task_input_instance(
        &self,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<Vec<u8>, ExpressionFailure> {
        Ok(self.task_input(data, instance.node()))
    }
}

#[tokio::test]
async fn internal_input_limit_failure_commits_even_with_vacant_and_busy_group() {
    crate::support::bounded::within(
        "oversized context never needs capacity possession",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            vacate(&n).await;
            let session = n.f.w.session(&["capacity.work"]).await;
            let workers = WorkerService::new(
                PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone()),
                SystemClock,
                Engine::new(OversizedTaskContext),
                SystemEntropy,
                WorkerSettings::default(),
            ).with_child_root_capacity_guard();
            let gate = group_lock(&n).await;
            let tasks = tokio::time::timeout(Duration::from_secs(5), workers.poll(
                &session.principal, session.worker_id, &session.token, 1, None,
            )).await.expect("internal failure never waits on G").expect("preflight failure commits");
            assert!(tasks.is_empty(), "no oversized external lease");
            let step: (String, Option<String>, i32, Option<Vec<u8>>) = sqlx::query_as("SELECT status,failure_code,attempt,lease_token_hash FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='work'")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain)
                .fetch_one(&n.f.w.db.pool).await.expect("real internal failure projection");
            assert_eq!(step, ("FAILED".into(), Some("RUN_DATA_LIMIT".into()), 1, None));
            let leases: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND status IN ('LEASED','RUNNING')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).fetch_one(&n.f.w.db.pool).await.expect("no external occupancy");
            assert_eq!(leases, 0);
            assert_eq!(state(&n.f.w.db.pool, &n.f).await, (2,2,1,"VACANT".into(),"ACTIVE".into()));
            n.f.w.assert_consistent(RunId::new(n.f.chain)).await;
            gate.rollback().await.expect("end unchanged G barrier");
        },
    ).await;
}

#[tokio::test]
async fn scoped_internal_input_limit_failure_also_ignores_vacant_and_busy_group() {
    use sagat_application::execution::DefinitionCache;
    use std::sync::Arc;
    crate::support::bounded::within(
        "scoped oversized context never needs possession",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let f = super::super::super::scoped_worker::fixture("capacity-scoped-preflight").await;
            let (group, _) = seed_scoped_binding(&f).await;
            let runtime = f.world.db.runtime_pool_with(16).await;
            let cache = DefinitionCache::new(Arc::new(super::super::super::scoped_worker::StructuredJsonIrCodec), 8);
            let workers = WorkerService::new(
                PgRunStore::new(runtime, cache).with_scoped_worker_protocol(),
                SystemClock, Engine::new(OversizedTaskContext), SystemEntropy, WorkerSettings::default(),
            ).with_vault(f.world.keys.cipher()).with_child_root_capacity_guard();
            let mut gate = f.world.db.pool.begin().await.expect("unchanged scoped G barrier");
            sqlx::query("SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE")
                .bind(f.world.admin.tenant_id().as_uuid()).bind(group)
                .execute(&mut *gate).await.expect("hold scoped fixture G");
            let tasks = tokio::time::timeout(Duration::from_secs(5), workers.poll(
                &f.session.principal, f.session.worker_id, &f.session.token, 1, None,
            )).await.expect("internal scoped failure never waits on G").expect("scoped preflight failure commits");
            assert!(tasks.is_empty());
            let failed: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND cardinality(instance_path_nodes)>0 AND status='FAILED' AND failure_code='RUN_DATA_LIMIT' AND attempt=1 AND lease_token_hash IS NULL")
                .bind(f.world.admin.tenant_id().as_uuid()).bind(f.run.as_uuid())
                .fetch_one(&f.world.db.pool).await.expect("real scoped internal failure");
            assert!(failed > 0, "eligible scoped work progresses through input failure before G");
            let held: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status IN ('LEASED','RUNNING')")
                .bind(f.world.admin.tenant_id().as_uuid()).bind(f.run.as_uuid())
                .fetch_one(&f.world.db.pool).await.expect("no external scoped lease occupancy");
            assert_eq!(held, 0);
            gate.rollback().await.expect("end scoped G barrier");
        },
    ).await;
}

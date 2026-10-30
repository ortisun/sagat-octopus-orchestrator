//! O706: real worker polling with an opt-in ChildRoot possession prerequisite.
use super::*;
use sagat_application::{
    capacity_vacate::{CapacityVacateCommand, CurrentChildRootVacateStore},
    clock::SystemClock,
    entropy::SystemEntropy,
    execution::Engine,
    worker::{WorkerService, WorkerSettings},
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::{PgCapacityVacateStore, PgRunStore};
use std::time::Duration;

async fn guarded_fixture() -> NativeFixture {
    let mut n = super::barriers::runtime_fixture().await;
    n.f.w.workers = WorkerService::new(
        PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone()),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings {
            lease_ttl: Duration::from_secs(600),
            session_ttl: Duration::from_secs(600),
            ..WorkerSettings::default()
        },
    )
    .with_vault(n.f.w.keys.cipher())
    .with_child_root_capacity_guard();
    n
}

async fn vacate(n: &NativeFixture) {
    let c = native_command(n, RunId::new(n.f.chain)).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&CapacityVacateCommand {
            operation_key: c.operation_key,
            identity: c.identity,
            expected: c.expected,
            scheduler: c.scheduler,
        })
        .await
        .expect("real native Vacate commits before poll");
}

#[tokio::test]
async fn vacant_child_stays_ready_without_consuming_an_attempt_or_quota() {
    crate::support::bounded::within(
        "worker poll after ChildRoot Vacate",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let session = n.f.w.session(&["capacity.work"]).await;
            vacate(&n).await;
            let before = execution_snapshot(&n.f).await;
            let tuples = super::vacate::untouched(&n.f).await;
            assert!(
                n.f.w.poll(&session, 1).await.is_empty(),
                "vacancy grants no external lease"
            );
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(super::vacate::untouched(&n.f).await, tuples);
        },
    )
    .await;
}

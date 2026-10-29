//! Native dormant Acquire: real ChildRoot lineage, runtime role and retained receipts.

use super::*;
use sagat_application::capacity_acquire::{
    CapacityAcquireCommand, CapacityAcquireError as Error, CurrentChildRootAcquireStore,
};
use sagat_application::capacity_vacate::{CapacityVacateCommand, CurrentChildRootVacateStore};
use sagat_infra_postgres::{PgCapacityAcquireStore, PgCapacityVacateStore};

async fn command(n: &NativeFixture) -> CapacityAcquireCommand {
    let c = native_command(n, RunId::new(n.f.chain)).await;
    CapacityAcquireCommand {
        operation_key: c.operation_key,
        identity: c.identity,
        expected: c.expected,
        scheduler: c.scheduler,
    }
}

async fn vacant_fixture() -> NativeFixture {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&CapacityVacateCommand {
            operation_key: c.operation_key,
            identity: c.identity,
            expected: c.expected,
            scheduler: c.scheduler,
        })
        .await
        .expect("real quiescent ChildRoot Vacate precedes Acquire");
    n
}

async fn counts(f: &Fixture) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2 AND kind='ACQUIRE'),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$3 AND action='capacity.member.acquired'),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='capacity.member.acquired')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(f.group.to_string())
        .fetch_one(&f.w.db.pool).await.expect("Acquire side effects")
}

#[tokio::test]
async fn vacant_child_acquires_atomically_without_rewriting_execution_or_membership() {
    crate::support::bounded::within(
        "native ChildRoot Acquire apply",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let before = execution_snapshot(&n.f).await;
            let versions = super::vacate::untouched(&n.f).await;
            let result = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                .acquire_current_child_root(&c)
                .await
                .expect("vacant current ChildRoot acquires");
            assert_eq!(
                (
                    result.revision,
                    result.epoch,
                    result.active_members,
                    result.possession,
                    result.replayed
                ),
                (
                    3,
                    3,
                    1,
                    CapacityPossession::Held(CapacityMemberId::new(n.f.member)),
                    false
                )
            );
            assert_eq!(
                state(&n.f.w.db.pool, &n.f).await,
                (3, 3, 1, "HELD".into(), "ACTIVE".into())
            );
            assert_eq!(counts(&n.f).await, (1, 1, 1));
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(super::vacate::untouched(&n.f).await, versions);
        },
    )
    .await;
}

#[tokio::test]
async fn exact_replay_returns_the_original_acquisition_once() {
    crate::support::bounded::within(
        "native ChildRoot Acquire replay",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let a = store.acquire_current_child_root(&c).await.expect("acquire");
            let mut b = store
                .acquire_current_child_root(&c)
                .await
                .expect("receipt precedes occupied gate");
            assert!(b.replayed);
            b.replayed = false;
            assert_eq!(a, b);
            assert_eq!(counts(&n.f).await, (1, 1, 1));
        },
    )
    .await;
}

#[tokio::test]
async fn occupied_group_refuses_a_fresh_acquire_without_side_effects() {
    crate::support::bounded::within(
        "native ChildRoot Acquire occupied refusal",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = super::barriers::runtime_fixture().await;
            let c = command(&n).await;
            let before = execution_snapshot(&n.f).await;
            assert_eq!(
                PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                    .acquire_current_child_root(&c)
                    .await,
                Err(Error::Occupied)
            );
            assert_eq!(
                state(&n.f.w.db.pool, &n.f).await,
                (1, 1, 1, "HELD".into(), "ACTIVE".into())
            );
            assert_eq!(counts(&n.f).await, (0, 0, 0));
            assert_eq!(execution_snapshot(&n.f).await, before);
        },
    )
    .await;
}

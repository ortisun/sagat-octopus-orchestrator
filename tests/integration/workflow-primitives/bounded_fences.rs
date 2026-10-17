//! Real lock barriers for the authority held by exact-path execution transactions.

use sagat_application::execution::{PathExecutionStore, PathExecutionTx, RunTx};
use sagat_application::runs::{Lock, RunStore};
use sagat_application::shards::Lease;
use sagat_domain::execution::InstancePath;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::NodeId;

use crate::scoped_worker::fixture;
use crate::support::eventually;

fn first_path() -> InstancePath {
    InstancePath::root()
        .child(NodeId::parse("items").expect("scope owner"), 0)
        .expect("member path")
}

#[tokio::test]
async fn a_bounded_binding_read_holds_current_authority_until_commit() {
    let fixture = fixture("bounded-binding-fence").await;
    let snapshot = fixture
        .store
        .load_path(fixture.world.admin.tenant_id(), fixture.run, first_path())
        .await
        .expect("exact-path snapshot")
        .expect("run");
    let mut tx = fixture
        .store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("bounded transaction")
        .expect("run");
    assert!(
        !tx.bound_patterns(fixture.world.worker.subject().as_str())
            .await
            .expect("current bindings")
            .is_empty()
    );
    let key = format!(
        "binding:{}:{}:{}",
        fixture.world.admin.tenant_id(),
        fixture.world.namespace.id(),
        fixture.world.worker.subject(),
    );
    let mut contender = fixture.world.db.pool.acquire().await.expect("revoker");
    sqlx::query("SET lock_timeout='100ms'")
        .execute(&mut *contender)
        .await
        .expect("bounded revocation wait");
    let blocked = sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&key)
        .execute(&mut *contender)
        .await
        .expect_err("the observed binding authority lasts through commit");
    assert_eq!(
        blocked
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03")
    );
    tx.commit().await.expect("commit releases authority");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&key)
        .execute(&mut *contender)
        .await
        .expect("revocation can proceed after the commit");
}

#[tokio::test]
async fn a_bounded_scheduler_fence_rechecks_expiry_after_its_row_lock_wait() {
    assert_expiry_after_shard_lock(true).await;
}

#[tokio::test]
async fn a_full_scheduler_fence_rechecks_expiry_after_its_row_lock_wait() {
    assert_expiry_after_shard_lock(false).await;
}

async fn assert_expiry_after_shard_lock(bounded: bool) {
    let fixture = fixture("scheduler-fence-clock").await;
    let shard = Shard::of(fixture.run.as_uuid());
    let (owner, generation): (String, i64) = sqlx::query_as(
        "UPDATE scheduler_shards SET expires_at=clock_timestamp()+interval '3 seconds' \
         WHERE shard=$1 RETURNING owner,generation",
    )
    .bind(i16::from(shard.index()))
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("visible lease expiry");
    let lease = Lease {
        shard,
        generation: u64::try_from(generation).expect("generation"),
    };
    let mut guard = fixture
        .world
        .db
        .pool
        .begin()
        .await
        .expect("shard lock guard");
    sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=$1 FOR UPDATE")
        .bind(i16::from(shard.index()))
        .execute(&mut *guard)
        .await
        .expect("hold shard row without modifying it");
    let store = fixture.store.clone();
    let tenant = fixture.world.admin.tenant_id();
    let run = fixture.run;
    let waiter = tokio::spawn(async move {
        if bounded {
            let snapshot = store
                .load_path(tenant, run, first_path())
                .await
                .expect("snapshot")
                .expect("run");
            let mut tx = store
                .begin_path(snapshot, Lock::Wait)
                .await
                .expect("bounded mutation")
                .expect("run");
            let held = tx.holds_shard(&owner, lease).await.expect("bounded fence");
            tx.commit().await.expect("finish bounded fence read");
            held
        } else {
            let mut tx = store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("full mutation")
                .expect("run");
            let held = tx.holds_shard(&owner, lease).await.expect("full fence");
            tx.commit().await.expect("finish full fence read");
            held
        }
    });
    eventually("the scheduler fence actually waits on the shard row", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
             AND state='active' AND wait_event_type='Lock' AND query LIKE '%FROM scheduler_shards%' \
             AND (NOT $1 OR usename='sagat_app'))",
        )
        .bind(crate::support::as_runtime_role())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("database lock barrier")
        .then_some(())
    })
    .await;
    eventually(
        "the visible scheduler lease expires while its row stays locked",
        || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT clock_timestamp()>=expires_at FROM scheduler_shards WHERE shard=$1",
            )
            .bind(i16::from(shard.index()))
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("database expiry barrier")
            .then_some(())
        },
    )
    .await;
    guard.rollback().await.expect("release unchanged shard row");
    assert!(
        !waiter.await.expect("fence waiter"),
        "waiting for a row lock must not authorize an already expired shard lease"
    );
}

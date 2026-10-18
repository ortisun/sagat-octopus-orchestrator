//! Dormant capacity metadata only. Owner fixtures establish referential shape, not production
//! ChildRoot admission, physical child authority, quota reservation or execution eligibility.

use crate::support::engine::{World, chain, world};
use sagat_domain::execution::RunId;
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    w: World,
    group: Uuid,
    member: Uuid,
}

async fn seed_chain(w: &World, name: &str) -> RunId {
    let (workflow, version) = w.publish(chain(name)).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let mut tx = w.db.pool.begin().await.expect("chain fixture transaction");
    sqlx::query(
        "INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,\
         definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) \
         VALUES($1,$2,$3,$3,$4,$5,2,$6,$7,$7)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(version.as_uuid())
    .bind(w.admin.subject().as_str())
    .bind(crate::support::now() + time::Duration::hours(1))
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("existing chain identity fixture");
    sqlx::query(
        "INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) \
         VALUES($1,$2,$3,0,$3,$4)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("existing segment fixture");
    tx.commit().await.expect("chain fixture commits");
    run
}

async fn fixture() -> Fixture {
    let w = world().await;
    let run = seed_chain(&w, "capacity-schema").await;
    let group = Uuid::new_v4();
    let member = Uuid::new_v4();
    let mut tx = w.db.pool.begin().await.expect("owner capacity fixture");
    sqlx::query(
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) \
         VALUES($1,$2,1,1,1,'HELD',$3)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(group)
    .bind(member)
    .execute(&mut *tx)
    .await
    .expect("owner-only dormant group fixture");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) \
         VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(group)
    .bind(member)
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("owner-only dormant binding fixture");
    tx.commit().await.expect("deferred active holder resolves");
    Fixture { w, group, member }
}

async fn state(pool: &PgPool, f: &Fixture) -> (i64, i64, i64, String, String) {
    sqlx::query_as(
        "SELECT g.revision,g.epoch,g.active_members,g.possession,m.state \
         FROM capacity_groups g JOIN capacity_group_members m USING(tenant_id,group_id) \
         WHERE g.tenant_id=$1 AND g.group_id=$2 AND m.member_id=$3",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .bind(f.member)
    .fetch_one(pool)
    .await
    .expect("exact fixture state")
}

#[tokio::test]
async fn valid_owner_fixture_precedes_capacity_constraint_probes() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool().await;
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("actual runtime identity");
    assert_eq!(user, "sagat_app");
    assert_eq!(
        state(&runtime, &f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
}

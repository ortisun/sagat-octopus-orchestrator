//! Database invariants for chain metadata before continuation handoff is enabled.

use crate::support::engine::{chain, world};
use sagat_application::runs::RunStore;

#[tokio::test]
async fn chain_head_must_be_a_segment_of_its_own_chain() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("chain-head-boundary")).await;
    let head = w.start(workflow, &serde_json::json!({})).await;
    let other = w.start(workflow, &serde_json::json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let deadline = crate::support::now() + time::Duration::hours(1);

    let mut tx = w.db.pool.begin().await.expect("transaction");
    sqlx::query(
        "INSERT INTO execution_chains (tenant_id, namespace_id, chain_id, current_run_id, \
         definition_version_id, started_by, max_segments, original_deadline_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,2,$7,$8,$8)",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(head.as_uuid())
    .bind(head.as_uuid())
    .bind(version.as_uuid())
    .bind(w.admin.subject().as_str())
    .bind(deadline)
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("chain");
    sqlx::query(
        "INSERT INTO chain_segments (tenant_id, namespace_id, chain_id, segment_index, run_id, created_at) \
         VALUES ($1,$2,$3,0,$4,$5)",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(head.as_uuid())
    .bind(head.as_uuid())
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("initial segment");
    tx.commit().await.expect("valid chain commits");

    let graph = w
        .store
        .graph(w.admin.tenant_id(), head)
        .await
        .expect("chain graph read")
        .expect("visible run graph");
    let link = graph.chain.expect("committed chain link");
    assert_eq!(link.chain_id, head);
    assert_eq!(link.current_run_id, head);
    assert_eq!(link.segment_index, 0);
    assert_eq!(link.segment_count, 1);
    assert_eq!(link.predecessor_run_id, None);
    assert_eq!(link.successor_run_id, None);

    let invalid = sqlx::query(
        "UPDATE execution_chains SET current_run_id=$1 WHERE tenant_id=$2 AND chain_id=$3",
    )
    .bind(other.as_uuid())
    .bind(tenant)
    .bind(head.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("head outside chain must be rejected");
    let (code, constraint) = crate::support::db_error(invalid);
    assert_eq!(code, "23503");
    assert_eq!(
        constraint.as_deref(),
        Some("execution_chains_head_segment_fk")
    );

    let stored: uuid::Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(tenant)
    .bind(head.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("committed head");
    assert_eq!(stored, head.as_uuid());

    let immutable = sqlx::query(
        "UPDATE chain_segments SET run_id=$1 WHERE tenant_id=$2 AND chain_id=$3 AND segment_index=0",
    )
    .bind(other.as_uuid())
    .bind(tenant)
    .bind(head.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("segment identity must not change");
    let (code, _) = crate::support::db_error(immutable);
    assert_eq!(code, "23514");

    let input: Vec<u8> =
        sqlx::query_scalar("SELECT input_digest FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(head.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("input digest");
    let foreign_successor = sqlx::query(
        "UPDATE chain_segments SET completion_kind='CONTINUED', successor_run_id=$1, \
         transfer_input_digest=$2, completed_at=$3 \
         WHERE tenant_id=$4 AND chain_id=$5 AND segment_index=0",
    )
    .bind(other.as_uuid())
    .bind(input)
    .bind(crate::support::now())
    .bind(tenant)
    .bind(head.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("successor outside chain must be rejected");
    let (code, constraint) = crate::support::db_error(foreign_successor);
    assert_eq!(code, "23503");
    assert_eq!(
        constraint.as_deref(),
        Some("chain_segments_successor_member_fk")
    );
}

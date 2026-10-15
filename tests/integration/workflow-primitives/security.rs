//! Tenant-bound storage evidence for the additive execution identity.
//!
//! Cross-run primitive admission and byte/retention quotas remain pending until their
//! application API and transactional storage contract are implemented.

use sagat_application::error::AppError;
use sagat_application::runs::RunStore;
use serde_json::json;
use uuid::Uuid;

use crate::support::engine::{chain, ctx, world};
use crate::support::eventually;

#[tokio::test]
async fn scoped_identity_storage_rejects_foreign_tenant_keys_and_reads_look_missing() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("tenant-bound-instance")).await;
    let run = w.start(workflow, &json!({})).await;
    let owner = w.admin.tenant_id();
    let foreign = w.other_admin.tenant_id();

    let owner_graph = w
        .runs
        .graph(&w.admin, run, &ctx())
        .await
        .expect("owner graph");
    assert!(!owner_graph.steps.is_empty());
    let root_only: bool = sqlx::query_scalar(
        "SELECT bool_and(cardinality(instance_path_nodes) = 0 \
                         AND cardinality(instance_path_indices) = 0) \
         FROM steps WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(owner.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("root identity projection");
    assert!(
        root_only,
        "the current fixture is an old-compatible root run"
    );
    let foreign_read = w.runs.graph(&w.other_admin, run, &ctx()).await;
    let absent_read = w
        .runs
        .graph(
            &w.other_admin,
            sagat_domain::execution::RunId::generate(),
            &ctx(),
        )
        .await;
    assert_eq!(foreign_read.map(|_| ()), Err(AppError::NotFound));
    assert_eq!(absent_read.map(|_| ()), Err(AppError::NotFound));
    assert!(
        RunStore::graph(&w.store, foreign, run)
            .await
            .expect("tenant-filtered graph lookup")
            .is_none()
    );

    let foreign_step = sqlx::query(
        "INSERT INTO steps \
         SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
             'tenant_id', $3::uuid, 'step_id', $4::uuid, \
             'instance_path_nodes', jsonb_build_array('items'), \
             'instance_path_indices', jsonb_build_array(0) \
         ))).* \
         FROM steps s \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND cardinality(s.instance_path_nodes) = 0 \
         LIMIT 1",
    )
    .bind(owner.as_uuid())
    .bind(run.as_uuid())
    .bind(foreign.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect_err("a scoped step cannot point across the tenant/run boundary");
    assert_eq!(
        foreign_step
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    let foreign_event = sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, 999, 'step.released', 'a', '{}'::jsonb, 'foreign-fixture', \
                 clock_timestamp(), $3, ARRAY['items'], ARRAY[0])",
    )
    .bind(foreign.as_uuid())
    .bind(run.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect_err("a scoped journal event cannot point across the tenant/run boundary");
    assert_eq!(
        foreign_event
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    let foreign_rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = $2) + \
                (SELECT count(*) FROM run_events WHERE tenant_id = $1 AND run_id = $2)",
    )
    .bind(foreign.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("foreign storage count");
    assert_eq!(foreign_rows, 0);

    let audited = eventually("the cross-tenant run read is audited", || async {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records \
             WHERE tenant_id = $1 AND action = 'access.denied.cross_tenant' \
               AND resource_type = 'run' AND resource_id = $2",
        )
        .bind(foreign.as_uuid())
        .bind(run.to_string())
        .fetch_one(&w.db.pool)
        .await
        .expect("cross-tenant audit count");
        (count > 0).then_some(count)
    })
    .await;
    assert_eq!(
        audited, 1,
        "the denial is charged only to the caller tenant"
    );

    w.assert_consistent(run).await;
}

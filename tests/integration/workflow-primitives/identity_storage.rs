//! Additive storage for root-compatible scoped execution identity.

use sagat_application::execution::RunTx;
use sagat_application::execution::instance_step_id;
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::workflow::NodeId;
use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::support::engine::{World, chain, world};

/// The last migration before the scoped-instance storage foundation.
const BEFORE: i64 = 20_261_026_000_000;

type LegacyEvent = (i64, String, Option<String>, Value);
type UpgradedEvent = (i64, String, Option<String>, Value, Option<Uuid>);

async fn column_exists(pool: &sqlx::PgPool, table: &str, column: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .expect("column lookup")
}

pub(super) async fn insert_legacy_root_run(w: &World, name: &str) -> (RunId, WorkflowVersionId) {
    let (workflow, version) = w.publish(chain(name)).await;
    let run = RunId::generate();
    let tenant = w.admin.tenant_id().as_uuid();
    let input = b"{}";
    let input_digest = Sha256::digest(input).to_vec();
    let node = NodeId::parse("a").expect("fixture node");
    let step_id = instance_step_id(version, run, &ExecutionInstanceId::root(node));
    let mut tx = w.db.pool.begin().await.expect("legacy fixture transaction");

    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(&input_digest)
    .bind(input.as_slice())
    .execute(&mut *tx)
    .await
    .expect("legacy input");
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, created_at, updated_at, root_run_id) \
         VALUES ($1, $2, $3, $4, $5, 'RUNNING', 2, $6, 'legacy-fixture', \
                 clock_timestamp(), clock_timestamp(), $1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(&input_digest)
    .execute(&mut *tx)
    .await
    .expect("legacy run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1, $2, 1, 'run.started', '{\"entrypoint\":\"a\"}'::jsonb, \
                 'legacy-fixture', clock_timestamp())",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("legacy journal");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, occurred_at) \
         VALUES ($1, $2, 2, 'step.released', 'a', '{}'::jsonb, \
                 'legacy-fixture', clock_timestamp())",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("legacy node journal event");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, updated_at) \
         VALUES ($1, $2, 'a', $3, $4, 'READY', 'act.a', clock_timestamp(), clock_timestamp())",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(step_id)
    .bind(w.namespace.id().as_uuid())
    .execute(&mut *tx)
    .await
    .expect("legacy root step");
    tx.commit().await.expect("legacy fixture commit");
    (run, version)
}

#[tokio::test]
async fn upgrading_populated_storage_keeps_root_rows_and_does_not_rewrite_the_journal() {
    let w = world().await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("return to the legacy identity schema");
    assert!(!column_exists(&w.db.pool, "steps", "instance_path_nodes").await);

    let (run, _) = insert_legacy_root_run(&w, "legacy-root-identity").await;

    let before: Vec<LegacyEvent> = sqlx::query(
        "SELECT seq, event_type, node, detail FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("legacy journal")
    .into_iter()
    .map(|row| {
        (
            row.get("seq"),
            row.get("event_type"),
            row.get("node"),
            row.get("detail"),
        )
    })
    .collect();

    migrate(&w.db.pool)
        .await
        .expect("apply scoped identity migration");

    let roots: Vec<(Vec<String>, Vec<i32>)> = sqlx::query_as(
        "SELECT instance_path_nodes, instance_path_indices FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("root step identities");
    assert!(!roots.is_empty(), "the run has projected steps");
    assert!(
        roots
            .iter()
            .all(|(nodes, indices)| nodes.is_empty() && indices.is_empty()),
        "legacy steps are the empty root path"
    );

    let after: Vec<UpgradedEvent> = sqlx::query(
        "SELECT seq, event_type, node, detail, instance_id FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("upgraded journal")
    .into_iter()
    .map(|row| {
        (
            row.get("seq"),
            row.get("event_type"),
            row.get("node"),
            row.get("detail"),
            row.get("instance_id"),
        )
    })
    .collect();
    assert_eq!(
        before,
        after
            .iter()
            .map(|(seq, kind, node, detail, _)| (*seq, kind.clone(), node.clone(), detail.clone()))
            .collect::<Vec<_>>(),
        "the migration must not replace or reinterpret old events"
    );
    assert!(
        after
            .iter()
            .all(|(_, _, _, _, instance)| instance.is_none()),
        "old events retain their legacy null identity and replay as root"
    );

    for (table, column) in [
        ("event_deliveries", "instance_id"),
        ("run_sagas", "instance_id"),
        ("script_executions", "instance_id"),
    ] {
        assert!(
            column_exists(&w.db.pool, table, column).await,
            "{table}.{column}"
        );
    }
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn one_definition_node_can_have_distinct_tenant_scoped_instance_paths() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("scoped-storage")).await;
    let run = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    for index in [0_i32, 1_i32] {
        sqlx::query(
            "INSERT INTO steps \
             SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
                 'step_id', $4::text, \
                 'instance_path_nodes', jsonb_build_array('items'), \
                 'instance_path_indices', jsonb_build_array($5::integer) \
             ))).* \
             FROM steps s \
             WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = $3 \
               AND cardinality(s.instance_path_nodes) = 0",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind("a")
        .bind(Uuid::now_v7())
        .bind(index)
        .execute(&w.db.pool)
        .await
        .expect("a distinct scoped instance");
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("instance count");
    assert_eq!(count, 3, "root plus two scoped instances of node a");

    let duplicate = sqlx::query(
        "INSERT INTO steps \
         SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
             'step_id', $4::text, \
             'instance_path_nodes', jsonb_build_array('items'), \
             'instance_path_indices', jsonb_build_array(0) \
         ))).* \
         FROM steps s \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = $3 \
           AND cardinality(s.instance_path_nodes) = 0",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind("a")
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect_err("the same full logical identity is unique");
    assert_eq!(
        duplicate
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("23505")
    );

    for invalid_path in [
        "UPDATE steps SET instance_path_nodes = ARRAY[['items']], \
         instance_path_indices = ARRAY[[0]] \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND cardinality(instance_path_nodes) = 0",
        "UPDATE steps SET instance_path_nodes = '[0:0]={items}'::text[], \
         instance_path_indices = '[0:0]={0}'::integer[] \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND cardinality(instance_path_nodes) = 0",
    ] {
        let rejected = sqlx::query(invalid_path)
            .bind(tenant)
            .bind(run.as_uuid())
            .bind("a")
            .execute(&w.db.pool)
            .await
            .expect_err("the database rejects a noncanonical instance path");
        assert_eq!(
            rejected
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("23514")
        );
    }

    let primary_key: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname \
         FROM pg_constraint c \
         CROSS JOIN LATERAL unnest(c.conkey) WITH ORDINALITY AS k(attnum, position) \
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum \
         WHERE c.conrelid = 'steps'::regclass AND c.contype = 'p' \
         ORDER BY k.position",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("steps primary key");
    assert_eq!(
        primary_key,
        [
            "tenant_id",
            "run_id",
            "instance_path_nodes",
            "instance_path_indices",
            "node",
        ],
        "tenant is the first dimension of the full path identity"
    );
}

#[tokio::test]
async fn locked_run_restores_every_scoped_instance_of_the_same_node() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("scoped-restore")).await;
    let run = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    for index in [0_i32, 1_i32] {
        sqlx::query(
            "INSERT INTO steps \
             SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
                 'step_id', $3::text, \
                 'instance_path_nodes', jsonb_build_array('items'), \
                 'instance_path_indices', jsonb_build_array($4::integer) \
             ))).* \
             FROM steps s \
             WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'a' \
               AND cardinality(s.instance_path_nodes) = 0",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(Uuid::now_v7())
        .bind(index)
        .execute(&w.db.pool)
        .await
        .expect("scoped fixture");
    }

    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("locked run")
        .expect("run exists");
    let (_, state) = tx.parts();
    let node = NodeId::parse("a").expect("node");
    assert_eq!(state.instances().count(), 3);
    assert!(
        state
            .instance(&ExecutionInstanceId::root(node.clone()))
            .is_some()
    );
    for index in [0_u32, 1_u32] {
        let path = InstancePath::root()
            .child(NodeId::parse("items").expect("scope"), index)
            .expect("bounded path");
        assert!(
            state
                .instance(&ExecutionInstanceId::in_path(path, node.clone()))
                .is_some(),
            "scoped instance {index} is retained"
        );
    }
    drop(tx);

    let graph = w
        .store
        .graph(w.admin.tenant_id(), run)
        .await
        .expect("projected graph")
        .expect("run exists");
    assert_eq!(graph.steps.len(), 3);
    assert_eq!(
        graph
            .steps
            .iter()
            .map(|step| step.instance.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "graph projection retains each logical instance"
    );
}

#[tokio::test]
async fn old_worker_candidate_query_does_not_offer_scoped_ready_steps() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("scoped-claim-gate")).await;
    let run = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    sqlx::query(
        "INSERT INTO steps \
         SELECT (jsonb_populate_record(NULL::steps, to_jsonb(s) || jsonb_build_object( \
             'step_id', $3::text, \
             'instance_path_nodes', jsonb_build_array('items'), \
             'instance_path_indices', jsonb_build_array(0) \
         ))).* \
         FROM steps s \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'a' \
           AND cardinality(s.instance_path_nodes) = 0",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&w.db.pool)
    .await
    .expect("scoped ready fixture");
    sqlx::query(
        "UPDATE steps SET status = 'SUCCEEDED', routed = true \
         WHERE tenant_id = $1 AND run_id = $2 AND node = 'a' \
           AND cardinality(instance_path_nodes) = 0",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("leave scoped work as the only ready step");

    let candidates: Vec<Uuid> = sqlx::query_scalar(sagat_infra_postgres::CLAIMABLE_SQL)
        .bind(tenant)
        .bind(w.namespace.id().as_uuid())
        .bind(vec!["act.a".to_owned()])
        .bind(8_i64)
        .bind(4_i32)
        .fetch_all(&w.db.pool)
        .await
        .expect("root-only candidates");
    assert!(candidates.is_empty());
}

//! Current-process half of the frozen-old-producer Q5 integration proof.
//!
//! The separate phased driver described in the companion notes retains a disposable external database after
//! the exact `6c44a0e4` producer exits; this ignored test consumes its quiescent receipt. Static SQL
//! mutations below are corruption fixtures; they never manufacture successful legacy state.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use sagat_application::entropy::SystemEntropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, Engine, MAX_RUN_OUTPUT_BYTES, RunTx,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::{AddressedRunEvent, ExecutionInstanceId, RunEvent, RunId};
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{SHARDS, Shard};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgPool, PgRunStore, PoolConfig, connect, migrate};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{Acquire as _, Row as _};
use uuid::Uuid;

const PINNED_LEGACY_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";
const PROJECTION_WITNESS_SQL: &str = r#"WITH selected_runs AS (
  SELECT run.id FROM runs run WHERE run.tenant_id=$1 AND run.id=ANY($2::uuid[])
) SELECT jsonb_build_object(
  'runs',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM runs row_data JOIN selected_runs selected ON selected.id=row_data.id
    WHERE row_data.tenant_id=$1),
  'usage',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id),'[]'::jsonb)
    FROM run_instance_usage row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'scopes',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.owner_path_nodes,row_data.owner_path_indices,row_data.owner_node),'[]'::jsonb)
    FROM iteration_scopes row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node),'[]'::jsonb)
    FROM steps row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'events',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.seq),'[]'::jsonb)
    FROM run_events row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'item_sources',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM collection_scope_items row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'iterations',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM collection_iterations row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'checkpoints',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM iteration_checkpoints row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'payloads',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex')),'[]'::jsonb)
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT run.input_digest FROM runs run JOIN selected_runs selected ON selected.id=run.id WHERE run.tenant_id=$1
      UNION SELECT step.output_digest FROM steps step JOIN selected_runs selected ON selected.id=step.run_id WHERE step.tenant_id=$1
      UNION SELECT scope.snapshot_digest FROM iteration_scopes scope JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT scope.result_digest FROM iteration_scopes scope JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT item.item_digest FROM collection_scope_items item JOIN selected_runs selected ON selected.id=item.run_id WHERE item.tenant_id=$1
      UNION SELECT member.item_digest FROM collection_iterations member JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT member.result_digest FROM collection_iterations member JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint JOIN selected_runs selected ON selected.id=checkpoint.run_id WHERE checkpoint.tenant_id=$1
      UNION SELECT event.payload_digest FROM run_events event JOIN selected_runs selected ON selected.id=event.run_id WHERE event.tenant_id=$1)),
  'source_blobs',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.namespace_id,row_data.id),'[]'::jsonb)
    FROM blobs row_data WHERE row_data.tenant_id=$1 AND (row_data.namespace_id,row_data.id) IN (
      SELECT scope.namespace_id,scope.source_blob_id FROM iteration_scopes scope
      JOIN selected_runs selected ON selected.id=scope.run_id
      WHERE scope.tenant_id=$1 AND scope.source_blob_id IS NOT NULL)),
  'workflow_versions',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM workflow_versions row_data WHERE row_data.tenant_id=$1 AND row_data.id IN (
      SELECT run.workflow_version_id FROM runs run JOIN selected_runs selected ON selected.id=run.id WHERE run.tenant_id=$1)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'outbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM event_outbox row_data JOIN selected_runs selected ON selected.id=row_data.aggregate_id
    WHERE row_data.tenant_id=$1))"#;

const CLOSED_COLLECTION_SOURCE_SQL: &str = r#"WITH selected_scopes AS (
  SELECT scope.* FROM iteration_scopes scope
   WHERE scope.tenant_id=$1 AND scope.run_id=$2
     AND scope.owner_path_nodes='{}'::text[]
     AND scope.owner_path_indices='{}'::integer[]
     AND scope.owner_node=ANY($3::text[])
), selected_events AS (
  SELECT event.* FROM run_events event
   WHERE event.tenant_id=$1 AND event.run_id=$2 AND event.seq <= $4
) SELECT jsonb_build_object(
  'scopes',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.owner_node)
    FROM selected_scopes row_data),
  'steps',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node)
    FROM steps row_data WHERE row_data.tenant_id=$1 AND row_data.run_id=$2
      AND ((cardinality(row_data.instance_path_nodes)=0 AND row_data.node=ANY($3::text[]))
        OR row_data.instance_path_nodes[1]=ANY($3::text[]))),
  'item_sources',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_scope_items row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'iterations',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_iterations row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'checkpoints',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM iteration_checkpoints row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'events',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.seq)
    FROM selected_events row_data),
  'payloads',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex'))
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT scope.snapshot_digest FROM selected_scopes scope
      UNION SELECT scope.result_digest FROM selected_scopes scope
      UNION SELECT step.output_digest FROM steps step
        WHERE step.tenant_id=$1 AND step.run_id=$2
          AND ((cardinality(step.instance_path_nodes)=0 AND step.node=ANY($3::text[]))
            OR step.instance_path_nodes[1]=ANY($3::text[]))
      UNION SELECT item.item_digest FROM collection_scope_items item JOIN selected_scopes scope
        ON scope.tenant_id=item.tenant_id AND scope.run_id=item.run_id
       AND scope.scope_id=item.scope_id
      UNION SELECT member.result_digest FROM collection_iterations member JOIN selected_scopes scope
        ON scope.tenant_id=member.tenant_id AND scope.run_id=member.run_id
       AND scope.scope_id=member.scope_id
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint
        JOIN selected_scopes scope ON scope.tenant_id=checkpoint.tenant_id
         AND scope.run_id=checkpoint.run_id AND scope.scope_id=checkpoint.scope_id
      UNION SELECT event.payload_digest FROM selected_events event)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data
    WHERE row_data.tenant_id=$1 AND row_data.run_id=$2
      AND row_data.owner_node=ANY($3::text[]))
)"#;

#[derive(Debug)]
struct Receipt {
    source_commit: String,
    database_url: String,
    runtime_database_url: String,
    database_name: String,
    tenant: String,
    alias_namespace: String,
    overlay: String,
    missing_owner: String,
    wrong_namespace: String,
    bad_metadata: String,
    missing_payload: String,
    bad_hash: String,
    over_budget: String,
    producer_witness_sha256: String,
}

impl Receipt {
    fn parse(bytes: &[u8]) -> Self {
        let mut value: Value = serde_json::from_slice(bytes).expect("parse producer receipt");
        let object = value.as_object_mut().expect("receipt object");
        assert_eq!(object.len(), 14, "receipt has exactly the reviewed fields");
        let mut take = |name: &str| {
            object
                .remove(name)
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| panic!("receipt string: {name}"))
        };
        let receipt = Self {
            source_commit: take("source_commit"),
            database_url: take("database_url"),
            runtime_database_url: take("runtime_database_url"),
            database_name: take("database_name"),
            tenant: take("tenant"),
            alias_namespace: take("alias_namespace"),
            overlay: take("overlay"),
            missing_owner: take("missing_owner"),
            wrong_namespace: take("wrong_namespace"),
            bad_metadata: take("bad_metadata"),
            missing_payload: take("missing_payload"),
            bad_hash: take("bad_hash"),
            over_budget: take("over_budget"),
            producer_witness_sha256: take("producer_witness_sha256"),
        };
        assert!(object.is_empty(), "receipt has no unknown fields");
        receipt
    }
}

struct Phase {
    receipt: Receipt,
    pool: PgPool,
    store: PgRunStore,
    tenant: TenantId,
}

impl Phase {
    fn run(&self, text: &str) -> RunId {
        text.parse().expect("receipt run id")
    }
}

async fn phase() -> Phase {
    let path = std::env::var_os("SAGAT_Q5_PHASE_RECEIPT")
        .expect("the phased runner supplies SAGAT_Q5_PHASE_RECEIPT");
    let receipt = Receipt::parse(&std::fs::read(path).expect("read private producer receipt"));
    assert_eq!(receipt.source_commit, PINNED_LEGACY_COMMIT);
    assert!(
        receipt.database_name.len() == 34
            && receipt.database_name.starts_with("t_")
            && receipt.database_name[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "owned disposable database name"
    );
    let tenant = receipt.tenant.parse().expect("receipt tenant");
    let pool = connect(&PoolConfig::new(receipt.database_url.clone()))
        .await
        .expect("connect to the retained old-producer database");
    let actual_database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current database identity");
    assert_eq!(actual_database, receipt.database_name);
    assert_eq!(
        old_shape_witness(&pool, tenant, &receipt).await,
        receipt.producer_witness_sha256,
        "quiescent old projections changed before current migration"
    );
    migrate(&pool)
        .await
        .expect("migrate old database with current tree");
    assert_eq!(
        old_shape_witness(&pool, tenant, &receipt).await,
        receipt.producer_witness_sha256,
        "current migration changed the bounded old projection witness"
    );
    let store_pool = if std::env::var_os("SAGAT_TEST_AS_RUNTIME_ROLE").is_some() {
        connect(&PoolConfig::new(receipt.runtime_database_url.clone()))
            .await
            .expect("connect current execution store as sagat_app")
    } else {
        pool.clone()
    };
    let store = PgRunStore::new(store_pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8))
        .with_scoped_worker_protocol();
    Phase {
        receipt,
        pool,
        store,
        tenant,
    }
}

async fn projection_witness(pool: &PgPool, tenant: TenantId, runs: &[RunId]) -> String {
    let ids = runs.iter().map(|run| (*run).as_uuid()).collect::<Vec<_>>();
    let value: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(tenant.as_uuid())
        .bind(ids)
        .fetch_one(pool)
        .await
        .expect("bounded canonical current projection witness");
    let digest = Sha256::digest(serde_json::to_vec(&value).expect("canonical witness JSON"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn old_shape_witness(pool: &PgPool, tenant: TenantId, receipt: &Receipt) -> String {
    let ids = [
        &receipt.overlay,
        &receipt.missing_owner,
        &receipt.wrong_namespace,
        &receipt.bad_metadata,
        &receipt.missing_payload,
        &receipt.bad_hash,
        &receipt.over_budget,
    ]
    .map(|run| run.parse::<RunId>().expect("receipt run"));
    projection_witness(pool, tenant, &ids).await
}

async fn ledger_and_legacy(p: &Phase, run: RunId) -> (u64, u64) {
    let row = sqlx::query(
        "SELECT usage.logical_output_bytes,
         COALESCE(sum(octet_length(payload.content))
           FILTER (WHERE owner.output_digest IS NULL),0)::bigint AS legacy
         FROM run_instance_usage usage
         JOIN iteration_scopes scope
           ON scope.tenant_id=usage.tenant_id AND scope.run_id=usage.run_id
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id
           AND owner.instance_path_nodes='{}'::text[]
           AND owner.instance_path_indices='{}'::integer[] AND owner.node=scope.owner_node
         JOIN payloads payload
           ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest
         WHERE usage.tenant_id=$1 AND usage.run_id=$2
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[] AND scope.lifecycle='SUCCEEDED'
         GROUP BY usage.logical_output_bytes",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("logical byte totals");
    (
        u64::try_from(row.get::<i64, _>("logical_output_bytes")).expect("ledger"),
        u64::try_from(row.get::<i64, _>("legacy")).expect("legacy bytes"),
    )
}

async fn fingerprint(p: &Phase, run: RunId) -> String {
    projection_witness(&p.pool, p.tenant, &[run]).await
}

async fn aggregate(p: &Phase, run: RunId, owner: &str) -> Vec<u8> {
    sqlx::query_scalar(
        "SELECT payload.content FROM iteration_scopes scope JOIN payloads payload
         ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node=$3
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(&p.pool)
    .await
    .expect("legacy aggregate")
}

fn closed_collection_nodes() -> Vec<String> {
    (0..10).map(|index| format!("c{index}")).collect()
}

async fn closed_collection_source_witness(p: &Phase, run: RunId) -> String {
    let nodes = closed_collection_nodes();
    let cutoff: Option<i64> = sqlx::query_scalar(
        "SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2
           AND event_type='step.routed' AND node='c9'
           AND cardinality(instance_path_nodes)=0",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("closed source journal boundary");
    let value: Value = sqlx::query_scalar(CLOSED_COLLECTION_SOURCE_SQL)
        .bind(p.tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(nodes)
        .bind(cutoff.expect("c9 owner routing event"))
        .fetch_one(&p.pool)
        .await
        .expect("closed collection source witness");
    let digest = Sha256::digest(serde_json::to_vec(&value).expect("canonical source witness JSON"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn assert_closed_collection_source_shape(p: &Phase, run: RunId) {
    let nodes = closed_collection_nodes();
    let row = sqlx::query(
        "SELECT
           (SELECT count(*) FROM iteration_scopes scope
             WHERE scope.tenant_id=$1 AND scope.run_id=$2
               AND scope.owner_path_nodes='{}'::text[]
               AND scope.owner_path_indices='{}'::integer[]
               AND scope.owner_node=ANY($3::text[]) AND scope.lifecycle='SUCCEEDED') AS scopes,
           (SELECT count(*) FROM collection_iterations member JOIN iteration_scopes scope
             ON scope.tenant_id=member.tenant_id AND scope.run_id=member.run_id
            AND scope.scope_id=member.scope_id
             WHERE scope.tenant_id=$1 AND scope.run_id=$2
               AND scope.owner_path_nodes='{}'::text[]
               AND scope.owner_path_indices='{}'::integer[]
               AND scope.owner_node=ANY($3::text[]) AND member.lifecycle='CHECKPOINTED') AS members,
           (SELECT count(*) FROM iteration_checkpoints checkpoint JOIN iteration_scopes scope
             ON scope.tenant_id=checkpoint.tenant_id AND scope.run_id=checkpoint.run_id
            AND scope.scope_id=checkpoint.scope_id
             WHERE scope.tenant_id=$1 AND scope.run_id=$2
               AND scope.owner_path_nodes='{}'::text[]
               AND scope.owner_path_indices='{}'::integer[]
               AND scope.owner_node=ANY($3::text[])) AS checkpoints,
           (SELECT count(*) FROM steps owner WHERE owner.tenant_id=$1 AND owner.run_id=$2
               AND owner.instance_path_nodes='{}'::text[]
               AND owner.instance_path_indices='{}'::integer[]
               AND owner.node=ANY($3::text[]) AND owner.status='SUCCEEDED'
               AND owner.output_digest IS NULL) AS legacy_owners,
           (SELECT count(*) FROM leaf_collection_success_certifications certificate
             WHERE certificate.tenant_id=$1 AND certificate.run_id=$2) AS certifications",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(nodes)
    .fetch_one(&p.pool)
    .await
    .expect("closed collection source shape");
    assert_eq!(
        (
            row.get::<i64, _>("scopes"),
            row.get::<i64, _>("members"),
            row.get::<i64, _>("checkpoints"),
            row.get::<i64, _>("legacy_owners"),
            row.get::<i64, _>("certifications"),
        ),
        (10, 10, 10, 10, 0),
        "the frozen producer made ten successful legacy collection sources and no certificates"
    );
}

async fn overlay_case(p: &Phase) {
    let run = p.run(&p.receipt.overlay);
    let shape = sqlx::query(
        "SELECT count(*)::bigint AS owners,
          count(DISTINCT encode(scope.result_digest,'hex'))::bigint AS digests,
          count(*) FILTER (WHERE owner.output_digest IS NULL)::bigint AS missing
         FROM iteration_scopes scope JOIN steps owner
           ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id
          AND owner.instance_path_nodes='{}'::text[]
          AND owner.instance_path_indices='{}'::integer[] AND owner.node=scope.owner_node
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.lifecycle='SUCCEEDED'
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("genuine legacy shape");
    assert_eq!(shape.get::<i64, _>("owners"), 2);
    assert_eq!(shape.get::<i64, _>("digests"), 1);
    assert_eq!(shape.get::<i64, _>("missing"), 2);
    let later_lease: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2
         AND status='LEASED' AND cardinality(instance_path_nodes)>0",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("later lease");
    assert_eq!(later_lease, 1);

    let before = fingerprint(p, run).await;
    let (ledger, legacy) = ledger_and_legacy(p, run).await;
    let effective = ledger.checked_add(legacy).expect("effective bytes");
    let c0 = aggregate(p, run, "c0").await;
    let c1 = aggregate(p, run, "c1").await;
    assert_eq!(c0, c1);
    let mut tx = p
        .store
        .begin(p.tenant, run, Lock::Wait)
        .await
        .expect("open run")
        .expect("stored run");
    assert_eq!(tx.logical_output_bytes().await.expect("scalar"), effective);
    let sum = {
        let data = tx.data().await.expect("overlay full data");
        assert_eq!(
            data.root_output(&NodeId::parse("c0").expect("c0")),
            Some(c0.as_slice())
        );
        assert_eq!(
            data.root_output(&NodeId::parse("c1").expect("c1")),
            Some(c1.as_slice())
        );
        data.outputs.values().map(Vec::len).sum::<usize>() as u64
    };
    assert_eq!(sum, effective);
    tx.commit().await.expect("read-only commit");
    assert_eq!(
        fingerprint(p, run).await,
        before,
        "the overlay is not backfilled"
    );
}

async fn expect_internal_scalar(p: &Phase, run: RunId, expected: &str) {
    let opened = p.store.begin(p.tenant, run, Lock::Wait).await;
    let error = match opened {
        Err(error) => error,
        Ok(Some(mut tx)) => tx
            .logical_output_bytes()
            .await
            .expect_err("corruption rejected"),
        Ok(None) => panic!("stored corruption fixture disappeared"),
    };
    match error {
        StoreError::Internal { detail } => {
            assert_eq!(detail, expected);
            assert!(detail.len() < 80, "fixed safe corruption detail");
        }
        other => panic!("expected internal corruption, got {other:?}"),
    }
}

async fn metadata_corruption_case(p: &Phase) {
    let wrong_namespace = p.run(&p.receipt.wrong_namespace);
    sqlx::query(
        "UPDATE iteration_scopes SET namespace_id=$3
         WHERE tenant_id=$1 AND run_id=$2 AND owner_node='c0'
           AND owner_path_nodes='{}'::text[] AND owner_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(wrong_namespace.as_uuid())
    .bind(Uuid::parse_str(&p.receipt.alias_namespace).expect("alias namespace"))
    .execute(&p.pool)
    .await
    .expect("static contradictory-namespace fixture");
    expect_internal_scalar(
        p,
        wrong_namespace,
        "stored successful collection namespace violates a domain invariant",
    )
    .await;

    let missing_owner = p.run(&p.receipt.missing_owner);
    let mut connection = p.pool.acquire().await.expect("fixture connection");
    let mut transaction = connection.begin().await.expect("fixture transaction");
    sqlx::query("SET LOCAL session_replication_role=replica")
        .execute(&mut *transaction)
        .await
        .expect("fixture-only constraint bypass");
    sqlx::query(
        "UPDATE steps SET node='orphaned-c0' WHERE tenant_id=$1 AND run_id=$2 AND node='c0'
         AND instance_path_nodes='{}'::text[] AND instance_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(missing_owner.as_uuid())
    .execute(&mut *transaction)
    .await
    .expect("static missing-owner fixture");
    transaction.commit().await.expect("commit fixture");
    expect_internal_scalar(
        p,
        missing_owner,
        "stored collection projection violates a domain invariant",
    )
    .await;

    let bad_metadata = p.run(&p.receipt.bad_metadata);
    sqlx::query(
        "UPDATE steps SET routed=false WHERE tenant_id=$1 AND run_id=$2 AND node='c0'
         AND instance_path_nodes='{}'::text[] AND instance_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(bad_metadata.as_uuid())
    .execute(&p.pool)
    .await
    .expect("static corrupt-owner fixture");
    expect_internal_scalar(
        p,
        bad_metadata,
        "stored successful collection output owner violates a domain invariant",
    )
    .await;
}

async fn mutate_payload(p: &Phase, run: RunId, statement: &'static str) {
    let mut connection = p.pool.acquire().await.expect("fixture connection");
    let mut transaction = connection.begin().await.expect("fixture transaction");
    sqlx::query("SET LOCAL session_replication_role=replica")
        .execute(&mut *transaction)
        .await
        .expect("fixture-only constraint bypass");
    // ALWAYS quota triggers must flush before ALTER restores the append-only trigger.
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *transaction)
        .await
        .expect("flush fixture quota triggers immediately");
    sqlx::query("ALTER TABLE payloads DISABLE TRIGGER payloads_append_only")
        .execute(&mut *transaction)
        .await
        .expect("disable append-only guard inside static corruption fixture");
    sqlx::query(statement)
        .bind(p.tenant.as_uuid())
        .bind(run.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("install static payload fixture");
    sqlx::query("ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_append_only")
        .execute(&mut *transaction)
        .await
        .expect("restore append-only guard inside static corruption fixture");
    transaction.commit().await.expect("commit payload fixture");
}

async fn payload_corruption_case(p: &Phase) {
    let missing = p.run(&p.receipt.missing_payload);
    mutate_payload(
        p,
        missing,
        "DELETE FROM payloads payload USING iteration_scopes scope
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='c0'
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[]
           AND payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest",
    )
    .await;
    expect_internal_scalar(
        p,
        missing,
        "stored collection output payload violates a domain invariant",
    )
    .await;

    let bad_hash = p.run(&p.receipt.bad_hash);
    mutate_payload(
        p,
        bad_hash,
        "UPDATE payloads payload SET content=set_byte(payload.content,0,
           CASE get_byte(payload.content,0) WHEN 91 THEN 123 ELSE 91 END)
         FROM iteration_scopes scope
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='c0'
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[]
           AND payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest",
    )
    .await;
    let (ledger, legacy) = ledger_and_legacy(p, bad_hash).await;
    let mut tx = p
        .store
        .begin(p.tenant, bad_hash, Lock::Wait)
        .await
        .expect("open bad-hash fixture")
        .expect("stored bad-hash fixture");
    assert_eq!(
        tx.logical_output_bytes().await.expect("metadata scalar"),
        ledger + legacy
    );
    match tx
        .data()
        .await
        .expect_err("digest mismatch rejected on hydration")
    {
        StoreError::Internal { detail } => assert_eq!(
            detail,
            "stored collection output payload violates a domain invariant"
        ),
        other => panic!("expected payload corruption, got {other:?}"),
    }
}

fn leased_instance(tx: &mut sagat_infra_postgres::PgRunTx) -> (ExecutionInstanceId, u32, u64) {
    tx.parts()
        .1
        .instances()
        .find(|(instance, step)| instance.node().as_str() == "work" && step.status().is_leased())
        .map(|(instance, step)| (instance.clone(), step.attempt(), step.generation()))
        .expect("later genuine scoped lease")
}

async fn persist_event(p: &Phase, run: RunId, output: Option<Vec<u8>>) -> Result<(), StoreError> {
    let mut tx = p
        .store
        .begin(p.tenant, run, Lock::Wait)
        .await?
        .expect("stored run");
    let (instance, attempt, generation) = leased_instance(&mut tx);
    let event = if output.is_some() {
        RunEvent::StepCompleted {
            node: instance.node().clone(),
            attempt,
            generation,
        }
    } else {
        RunEvent::StepStarted {
            node: instance.node().clone(),
            attempt,
            generation,
        }
    };
    let event = AddressedRunEvent::in_path(instance.path().clone(), event)
        .expect("address scoped lifecycle");
    tx.parts()
        .1
        .apply_addressed(&event)
        .expect("apply lifecycle");
    let outputs = output
        .map(|bytes| vec![(instance, bytes)])
        .unwrap_or_default();
    tx.persist_addressed(
        &[event],
        &outputs,
        Effects::by(Actor::System("q5-phased-test"), None),
    )
    .await?;
    tx.commit().await
}

async fn fitting_delta_case(p: &Phase) {
    let run = p.run(&p.receipt.overlay);
    let equal = serde_json::to_vec(&serde_json::json!({"same": true})).expect("equal output");
    let (ledger, legacy) = ledger_and_legacy(p, run).await;
    persist_event(p, run, None).await.expect("zero-delta write");
    assert_eq!(ledger_and_legacy(p, run).await, (ledger, legacy));
    persist_event(p, run, Some(equal.clone()))
        .await
        .expect("fitting positive write");
    let (after, same_legacy) = ledger_and_legacy(p, run).await;
    assert_eq!(after, ledger + equal.len() as u64);
    assert_eq!(same_legacy, legacy);
    let mut tx = p
        .store
        .begin(p.tenant, run, Lock::Wait)
        .await
        .expect("reopen")
        .expect("stored run");
    assert_eq!(
        tx.logical_output_bytes().await.expect("effective bytes"),
        after + legacy
    );
}

async fn over_budget_case(p: &Phase) {
    let run = p.run(&p.receipt.over_budget);
    let (ledger, legacy) = ledger_and_legacy(p, run).await;
    let effective = ledger.checked_add(legacy).expect("effective bytes");
    assert!(effective > MAX_RUN_OUTPUT_BYTES as u64);
    let mut tx = p
        .store
        .begin(p.tenant, run, Lock::Wait)
        .await
        .expect("open over-budget run")
        .expect("stored run");
    assert_eq!(
        tx.logical_output_bytes().await.expect("unclamped scalar"),
        effective
    );
    assert_eq!(
        tx.data().await.expect_err("full hydration refused"),
        StoreError::QuotaExceeded {
            what: "output retained by one run"
        }
    );
    drop(tx);

    persist_event(p, run, None)
        .await
        .expect("zero-delta write remains available");
    let after_zero: i64 = sqlx::query_scalar(
        "SELECT logical_output_bytes FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("ledger after zero delta");
    assert_eq!(u64::try_from(after_zero).expect("ledger"), ledger);

    assert_eq!(
        persist_event(p, run, Some(b"{}".to_vec()))
            .await
            .expect_err("positive delta refused"),
        StoreError::QuotaExceeded {
            what: "output retained by one run"
        }
    );
    let row = sqlx::query(
        "SELECT usage.logical_output_bytes,step.status FROM run_instance_usage usage
         JOIN steps step ON step.tenant_id=usage.tenant_id AND step.run_id=usage.run_id
         WHERE usage.tenant_id=$1 AND usage.run_id=$2 AND step.node='work'
           AND step.status='RUNNING' AND cardinality(step.instance_path_nodes)>0",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("positive write rolled back");
    assert_eq!(row.get::<i64, _>("logical_output_bytes"), ledger as i64);
    assert_eq!(row.get::<String, _>("status"), "RUNNING");
}

async fn legacy_scheduler_case(p: &Phase) {
    let run = p.run(&p.receipt.over_budget);
    let (ledger, legacy) = ledger_and_legacy(p, run).await;
    assert_eq!((ledger, legacy), (2_100_000, 2_100_020));
    assert!(ledger + legacy > MAX_RUN_OUTPUT_BYTES as u64);
    assert_closed_collection_source_shape(p, run).await;
    let source_before = closed_collection_source_witness(p, run).await;

    let authority = sqlx::query(
        "SELECT count(*) FILTER (WHERE owner IS NOT NULL)::bigint AS assigned,
                count(DISTINCT owner) FILTER (WHERE owner IS NOT NULL)::bigint AS owners,
                min(owner) AS first_owner,max(owner) AS last_owner
           FROM scheduler_shards",
    )
    .fetch_one(&p.pool)
    .await
    .expect("frozen producer scheduler authority");
    let old_owner = authority
        .get::<Option<String>, _>("first_owner")
        .expect("recorded old scheduler owner");
    assert_eq!(
        authority.get::<Option<String>, _>("last_owner").as_deref(),
        Some(old_owner.as_str())
    );
    assert_eq!(old_owner, "scope-compensation-q5-frozen-legacy-output");
    assert_eq!(authority.get::<i64, _>("assigned"), i64::from(SHARDS));
    assert_eq!(authority.get::<i64, _>("owners"), 1);
    // The coordinator waits for the old process to exit. Its stable owner may renew
    // still-valid database leases; process exit itself does not expire those leases.

    let placement = Placement::new(old_owner);
    let keeper = Keeper::new(
        p.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    );
    let round = keeper
        .round()
        .await
        .expect("resume the old scheduler owner through the normal keeper");
    assert_eq!(round.replicas, 1);
    assert_eq!(round.held, usize::from(SHARDS));
    assert!(placement.owned().contains(Shard::of(run.as_uuid())));

    let scheduler = Scheduler::new(
        p.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let lease_remaining_ms: i64 = sqlx::query_scalar(
        "SELECT ceil(extract(epoch FROM (work.lease_expires_at-clock_timestamp()))*1000)::bigint
           FROM steps work WHERE work.tenant_id=$1 AND work.run_id=$2 AND work.node='work'
             AND work.instance_path_nodes=ARRAY['c10']::text[]
             AND work.instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("the genuine old worker lease deadline");
    assert!(
        lease_remaining_ms > 0,
        "the discovery proof requires a still-live worker lease"
    );
    let started = scheduler.tick().await;
    assert_eq!(
        started.failed, 0,
        "normal existing-output drain: {started:?}"
    );
    let draining: (String, Option<String>, Option<String>, bool, String) = sqlx::query_as(
        "SELECT scope.lifecycle,scope.drain_kind,run.stop_code,work.cancel_requested,work.status
           FROM iteration_scopes scope JOIN runs run
             ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id
           JOIN steps work ON work.tenant_id=scope.tenant_id AND work.run_id=scope.run_id
             AND work.node='work' AND work.instance_path_nodes=ARRAY['c10']::text[]
             AND work.instance_path_indices=ARRAY[0]::integer[]
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='c10'
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[]",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("actual discovery starts a drain before worker expiry");
    assert_eq!(
        draining,
        (
            "DRAINING".to_owned(),
            Some("RUN_DATA_LIMIT".to_owned()),
            Some("RUN_DATA_LIMIT".to_owned()),
            true,
            "RUNNING".to_owned()
        )
    );
    // The old worker process exited, but its real lease remains authoritative until expiry.
    // Observe the recorded deadline; normal scheduler maintenance then settles the lost worker.
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(lease_remaining_ms).expect("positive lease remainder") + 25,
    ))
    .await;
    let mut finished = false;
    for _ in 0..16 {
        let report = scheduler.tick().await;
        assert_eq!(
            report.failed, 0,
            "all scheduler fixtures are valid at this point"
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(p.tenant.as_uuid())
                .bind(run.as_uuid())
                .fetch_one(&p.pool)
                .await
                .expect("target run status");
        if status == "FAILED" {
            finished = true;
            break;
        }
    }
    assert!(
        finished,
        "the bounded scheduler did not finish the target run"
    );

    let row = sqlx::query(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code,
                run.result_digest IS NULL AS no_run_result,
                scope.lifecycle,scope.drain_kind,scope.drain_failure_code,
                scope.result_digest IS NULL AS no_scope_result,
                scope.finish_sequence,scope.finish_outbox_id,
                owner.status AS owner_status,owner.failure_code AS owner_failure,
                owner.output_digest IS NULL AS no_owner_output,
                member.lifecycle AS member_lifecycle,
                work.status AS work_status,work.output_digest IS NULL AS no_work_output,
                (SELECT count(*) FROM steps scoped WHERE scoped.tenant_id=run.tenant_id
                  AND scoped.run_id=run.id AND scoped.instance_path_nodes[1]='c10') AS scoped_steps,
                (SELECT count(*) FROM steps root_owner WHERE root_owner.tenant_id=run.tenant_id
                  AND root_owner.run_id=run.id AND root_owner.instance_path_nodes='{}'::text[]
                  AND root_owner.node=ANY($3::text[]) AND root_owner.output_digest IS NOT NULL)
                  AS closed_owner_outputs,
                usage.logical_output_bytes
           FROM runs run JOIN run_instance_usage usage
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id
             AND scope.owner_path_nodes='{}'::text[]
             AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'
           JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id
             AND owner.instance_path_nodes='{}'::text[]
             AND owner.instance_path_indices='{}'::integer[] AND owner.node='c10'
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id
             AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id
             AND member.iteration_index=0
           JOIN steps work ON work.tenant_id=run.tenant_id AND work.run_id=run.id
             AND work.instance_path_nodes=member.entry_path_nodes
             AND work.instance_path_indices=member.entry_path_indices AND work.node='work'
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(closed_collection_nodes())
    .fetch_one(&p.pool)
    .await
    .expect("native existing-output drain projections");
    assert_eq!(row.get::<String, _>("run_status"), "FAILED");
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("FAILED")
    );
    assert_eq!(
        row.get::<Option<String>, _>("stop_code").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_run_result"));
    assert_eq!(row.get::<String, _>("lifecycle"), "FAILED");
    assert_eq!(
        row.get::<Option<String>, _>("drain_kind").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        row.get::<Option<String>, _>("drain_failure_code")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_scope_result"));
    assert!(row.get::<i64, _>("finish_sequence") > 0);
    assert_ne!(row.get::<Uuid, _>("finish_outbox_id"), Uuid::nil());
    assert_eq!(row.get::<String, _>("owner_status"), "FAILED");
    assert_eq!(
        row.get::<Option<String>, _>("owner_failure").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_owner_output"));
    assert_eq!(row.get::<String, _>("member_lifecycle"), "DRAINED");
    assert_eq!(row.get::<String, _>("work_status"), "CANCELLED");
    assert!(row.get::<bool, _>("no_work_output"));
    assert_eq!(
        row.get::<i64, _>("scoped_steps"),
        1,
        "no body successor was released"
    );
    assert_eq!(row.get::<i64, _>("closed_owner_outputs"), 0);
    assert_eq!(row.get::<i64, _>("logical_output_bytes"), ledger as i64);
    assert_eq!(ledger_and_legacy(p, run).await, (ledger, legacy));

    let journal = sqlx::query(
        "SELECT
           count(*) FILTER (WHERE event_type='run.stop_requested'
             AND detail->>'reason'='FAILED' AND detail->>'code'='RUN_DATA_LIMIT') AS stops,
           count(*) FILTER (WHERE event_type='collection.drain_started' AND node='c10'
             AND detail->>'reason'='run_data_limit') AS drains,
           count(*) FILTER (WHERE event_type='collection.iteration_drained' AND node='c10') AS members,
           count(*) FILTER (WHERE event_type='collection.finished' AND node='c10'
             AND detail->>'outcome'='failed' AND detail->>'code'='RUN_DATA_LIMIT') AS finishes,
           count(*) FILTER (WHERE event_type='run.finished'
             AND detail->>'status'='FAILED') AS run_finishes
         FROM run_events WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("truthful data-limit journal");
    assert_eq!(
        (
            journal.get::<i64, _>("stops"),
            journal.get::<i64, _>("drains"),
            journal.get::<i64, _>("members"),
            journal.get::<i64, _>("finishes"),
            journal.get::<i64, _>("run_finishes"),
        ),
        (1, 1, 1, 1, 1)
    );

    let outbox = sqlx::query(
        "SELECT
           count(*) FILTER (WHERE event_type='collection.drain.started'
             AND routing_key='c10' AND payload->>'reason'='FAILED') AS drains,
           count(*) FILTER (WHERE event_type='collection.finished'
             AND routing_key='c10' AND payload->>'status'='FAILED') AS finishes,
           count(*) FILTER (WHERE event_type='run.finished'
             AND payload->>'status'='FAILED') AS run_finishes
         FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("public failure hints");
    assert_eq!(
        (
            outbox.get::<i64, _>("drains"),
            outbox.get::<i64, _>("finishes"),
            outbox.get::<i64, _>("run_finishes"),
        ),
        (1, 1, 1)
    );

    let audit = sqlx::query(
        "SELECT count(*) AS records,max(audit.actor) AS actor,max(audit.reason) AS reason,
                bool_and(audit.trace_id IS NOT DISTINCT FROM run.trace_id) AS matching_trace
           FROM audit_records audit JOIN runs run
             ON run.tenant_id=audit.tenant_id AND run.id=$2
          WHERE audit.tenant_id=$1 AND audit.resource_type='run'
            AND audit.resource_id=$2::text AND audit.action='run.data_limit_exceeded'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.pool)
    .await
    .expect("private fixed-vocabulary refusal audit");
    assert_eq!(audit.get::<i64, _>("records"), 1);
    assert_eq!(
        audit.get::<Option<String>, _>("actor").as_deref(),
        Some("system:engine")
    );
    assert_eq!(
        audit.get::<Option<String>, _>("reason").as_deref(),
        Some("existing_outputs")
    );
    assert!(
        audit
            .get::<Option<bool>, _>("matching_trace")
            .unwrap_or(false)
    );

    assert_closed_collection_source_shape(p, run).await;
    assert_eq!(
        closed_collection_source_witness(p, run).await,
        source_before,
        "the native drain preserves the old closed source prefix and its payloads"
    );
    p.store
        .verify_run(p.tenant, run)
        .await
        .expect("the mixed old-source/current-drain history replays");
}

#[tokio::test]
#[ignore = "requires the frozen 6c44a0e4 producer phase; see companion notes"]
async fn a_current_full_run_transaction_reads_frozen_legacy_collection_outputs() {
    let p = phase().await;
    overlay_case(&p).await;
    over_budget_case(&p).await;
    fitting_delta_case(&p).await;
    legacy_scheduler_case(&p).await;
    metadata_corruption_case(&p).await;
    payload_corruption_case(&p).await;
}

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
use serde_json::{Value, json};
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

const NONFINAL_CLOSED_SOURCE_SQL: &str = CLOSED_COLLECTION_SOURCE_SQL;
fn nonfinal_sha(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("canonical private witness"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn nonfinal_source(pool: &sqlx::PgPool, tenant: Uuid, run: Uuid, cutoff: i64) -> String {
    let closed: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
        .bind(tenant)
        .bind(run)
        .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .expect("immutable closed old source");
    let target: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'version',(SELECT to_jsonb(v) FROM workflow_versions v WHERE v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id),
      'captured',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) FROM collection_scope_items i
          JOIN iteration_scopes s ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id AND s.scope_id=i.scope_id
          WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND s.owner_node='c10'),
      'work',(SELECT to_jsonb(w) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id
          AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='work'),
      'work_payload',(SELECT to_jsonb(p) FROM steps w JOIN payloads p ON p.tenant_id=w.tenant_id AND p.digest=w.output_digest
          WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes=ARRAY['c10']::text[]
            AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='work'),
      'work_events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e
          WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.instance_path_nodes=ARRAY['c10']::text[]
            AND e.instance_path_indices=ARRAY[0]::integer[] AND e.node='work'),
      'work_receipt',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.scope,c.idempotency_key) FROM command_dedupe c
          WHERE c.tenant_id=r.tenant_id AND c.scope LIKE $3)) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2"#)
        .bind(tenant).bind(run).bind(format!("step:{run}:scoped:3:c10:0;4:work:%"))
        .fetch_one(pool).await.expect("old target source and actual report receipt");
    assert!(
        target["work_receipt"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "one actual old forward report receipt"
    );
    nonfinal_sha(&json!({"closed":closed,"target":target}))
}

// Separate private handoff: old forward work is settled; only its nonfinal Return is pending.
struct NonfinalReceipt {
    database_name: String,
    database_url: String,
    runtime_url: String,
    tenant: TenantId,
    other_tenant: TenantId,
    namespace: Uuid,
    run: RunId,
    version: Uuid,
    scope: Uuid,
    owner: String,
    head: i64,
    cutoff: i64,
    forward: Uuid,
    target_key: String,
    projection: String,
    source: String,
}

impl NonfinalReceipt {
    fn parse(bytes: &[u8]) -> Self {
        let value: Value = serde_json::from_slice(bytes)
            .unwrap_or_else(|_| panic!("invalid private nonfinal handoff JSON"));
        let mut fields = value.as_object().expect("nonfinal handoff object").clone();
        assert!(
            fields.len() == 20,
            "closed twenty-field settled-work handoff"
        );
        let mut take = |name: &str| {
            fields
                .remove(name)
                .expect("required settled-work handoff field")
        };
        let text = |value: Value| {
            value
                .as_str()
                .expect("settled-work string field")
                .to_owned()
        };
        assert!(
            text(take("format")) == "legacy-nonfinal-return-frontier-v1"
                && text(take("source_commit")) == PINNED_LEGACY_COMMIT
                && text(take("case")) == "nonfinal-return",
            "exact private nonfinal format and immutable source"
        );
        let role = text(take("role"));
        assert!(
            role == if crate::support::as_runtime_role() {
                "runtime"
            } else {
                "owner"
            },
            "handoff and consumer roles match"
        );
        let database_name = text(take("database_name"));
        assert!(
            database_name.starts_with("t_")
                && database_name.len() == 34
                && database_name[2..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "exact owned database name"
        );
        let receipt = Self {
            database_name,
            database_url: text(take("database_url")),
            runtime_url: text(take("runtime_database_url")),
            tenant: text(take("tenant")).parse().expect("old tenant id"),
            other_tenant: text(take("other_tenant"))
                .parse()
                .expect("old other tenant id"),
            namespace: text(take("namespace")).parse().expect("old namespace id"),
            run: text(take("run")).parse().expect("old run id"),
            version: text(take("workflow_version"))
                .parse()
                .expect("old pinned version"),
            scope: text(take("scope_id")).parse().expect("old target scope"),
            owner: text(take("scheduler_owner")),
            head: take("handoff_run_version")
                .as_i64()
                .expect("old journal head"),
            cutoff: take("closed_source_cutoff")
                .as_i64()
                .expect("old closed prefix cutoff"),
            forward: text(take("forward_step_id"))
                .parse()
                .expect("old completed work id"),
            target_key: text(take("forward_target_key")),
            projection: text(take("producer_witness_sha256")),
            source: text(take("closed_source_sha256")),
        };
        assert!(
            fields.is_empty()
                && receipt.owner == format!("nonfinal-return-{role}")
                && receipt.head > receipt.cutoff
                && receipt.cutoff > 0
                && !receipt.target_key.is_empty()
                && receipt.target_key.len() <= 1024,
            "complete settled-work identity"
        );
        for hash in [&receipt.projection, &receipt.source] {
            assert!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "canonical source SHA256"
            );
        }
        receipt
    }
}

async fn nonfinal_bytes(pool: &PgPool, tenant: TenantId, run: RunId) -> (i64, i64) {
    sqlx::query_as("SELECT u.logical_output_bytes,(SELECT sum(octet_length(p.content))::bigint FROM iteration_scopes s JOIN steps o ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node AND cardinality(o.instance_path_nodes)=0 JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.lifecycle='SUCCEEDED' AND o.output_digest IS NULL) FROM run_instance_usage u WHERE u.tenant_id=$1 AND u.run_id=$2")
        .bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(pool).await.expect("actual logical and per-owner legacy bytes")
}

async fn nonfinal_state(pool: &PgPool, tenant: TenantId, run: RunId) -> Value {
    sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'run',to_jsonb(r),'scope',to_jsonb(s),'usage',to_jsonb(u),
      'members',(SELECT jsonb_agg(to_jsonb(m) ORDER BY m.iteration_index) FROM collection_iterations m WHERE m.tenant_id=r.tenant_id AND m.run_id=r.id AND m.scope_id=s.scope_id),
      'checkpoints',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY c.scope_id,c.iteration_index),'[]') FROM iteration_checkpoints c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'items',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) FROM collection_scope_items i WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND i.scope_id=s.scope_id),
      'steps',(SELECT jsonb_agg(to_jsonb(w) ORDER BY w.instance_path_nodes,w.instance_path_indices,w.node) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id),
      'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id),
      'outbox',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY o.id),'[]') FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id),
      'audits',(SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.id),'[]') FROM audit_records a WHERE a.tenant_id=r.tenant_id AND a.resource_id=r.id::text),
      'payloads',(SELECT jsonb_agg(to_jsonb(p) ORDER BY encode(p.digest,'hex')) FROM payloads p WHERE p.tenant_id=r.tenant_id),
      'receipts',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.scope,c.idempotency_key) FROM command_dedupe c WHERE c.tenant_id=r.tenant_id),
      'iteration_usage',(SELECT to_jsonb(t) FROM tenant_iteration_usage t WHERE t.tenant_id=r.tenant_id),
      'telemetry',(SELECT to_jsonb(t) FROM telemetry_pending t WHERE t.tenant_id=r.tenant_id AND t.run_id=r.id),
      'sagas',(SELECT coalesce(jsonb_agg(to_jsonb(g) ORDER BY g.owner_path_nodes,g.owner_path_indices,g.owner_node),'[]') FROM scope_sagas g WHERE g.tenant_id=r.tenant_id AND g.run_id=r.id),
      'saga_entries',(SELECT coalesce(jsonb_agg(to_jsonb(g) ORDER BY g.owner_path_nodes,g.owner_path_indices,g.owner_node,g.iteration_index,g.node),'[]') FROM scope_saga_entries g WHERE g.tenant_id=r.tenant_id AND g.run_id=r.id))
      FROM runs r JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id
      JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10'
      WHERE r.tenant_id=$1 AND r.id=$2"#).bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(pool).await.expect("bounded current nonfinal state")
}

fn nonfinal_step<'a>(state: &'a Value, node: &str) -> &'a Value {
    state["steps"]
        .as_array()
        .expect("step rows")
        .iter()
        .find(|s| {
            s["node"] == node
                && s["instance_path_nodes"] == serde_json::json!(["c10"])
                && s["instance_path_indices"] == serde_json::json!([0])
        })
        .expect("exact target scoped step")
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the separate immutable-old nonfinal Return producer; see fixture README"]
async fn current_frontier_drains_frozen_old_nonfinal_return_before_checkpoint() {
    use crate::support::{
        api::{Options, start_on_db},
        attach_existing_db,
        worker::{Platform, TENANT, Worker},
    };
    use sagat_application::scope_frontier::ScopeFrontierStore as _;
    use sagat_domain::execution::CollectionPhase;
    use sagat_scheduler::CollectionAdvance;
    assert!(
        std::env::var("SAGAT_Q5_OWNED_POSTGRES").as_deref() == Ok("1"),
        "only the root-owned repository PostgreSQL fixture"
    );
    let path = std::path::PathBuf::from(
        std::env::var_os("SAGAT_Q5_PHASE_RECEIPT").expect("private settled-work receipt"),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::metadata(&path)
                .expect("private receipt metadata")
                .permissions()
                .mode()
                & 0o777
                == 0o600,
            "private receipt permissions"
        );
    }
    let receipt =
        NonfinalReceipt::parse(&std::fs::read(&path).expect("read private settled-work handoff"));
    let admin = std::env::var("SAGAT_TEST_DATABASE_URL").expect("owned fixture admin URL");
    let db = attach_existing_db(&admin, &receipt.database_name).await;
    assert!(
        receipt.database_url == db.url
            && receipt.runtime_url
                == db.url_for(
                    crate::support::RUNTIME_ROLE,
                    crate::support::RUNTIME_ROLE_PASSWORD
                ),
        "receipt URLs are references to this exact owned fixture, never arbitrary targets"
    );
    for migrated in [false, true] {
        if migrated {
            migrate(&db.pool)
                .await
                .expect("current migration of genuine old data");
        }
        assert!(
            projection_witness(&db.pool, receipt.tenant, &[receipt.run]).await
                == receipt.projection,
            "migration and handoff preserve every old projection byte"
        );
        assert!(
            nonfinal_source(
                &db.pool,
                receipt.tenant.as_uuid(),
                receipt.run.as_uuid(),
                receipt.cutoff
            )
            .await
                == receipt.source,
            "validated old closed and target source stay exact"
        );
    }
    let api = start_on_db(
        db,
        Options {
            runtime_role: crate::support::as_runtime_role(),
            ..Options::default()
        },
    )
    .await;
    let platform = Platform {
        api,
        tenant: receipt.tenant,
        other_tenant: receipt.other_tenant,
    };
    let role: (String, String) =
        sqlx::query_as("SELECT current_user::text,current_database()::text")
            .fetch_one(&platform.api.api_pool)
            .await
            .expect("current API/store shared pool identity");
    let owner_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("current inspection owner");
    assert!(
        role.1 == receipt.database_name
            && role.0
                == if crate::support::as_runtime_role() {
                    crate::support::RUNTIME_ROLE
                } else {
                    owner_role.as_str()
                },
        "actual selected current API and execution-store role"
    );
    println!(
        "current_nonfinal_role api={} store={} database={}",
        role.0, role.0, role.1
    );
    let before = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    assert!(
        before["run"]["workflow_version_id"] == receipt.version.to_string()
            && before["run"]["namespace_id"] == receipt.namespace.to_string()
            && before["scope"]["scope_id"] == receipt.scope.to_string()
            && before["run"]["run_version"] == receipt.head,
        "exact original published and scope identities"
    );
    assert!(
        nonfinal_step(&before, "work")["step_id"] == receipt.forward.to_string(),
        "completed original public work identity"
    );
    assert!(
        nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == (2_100_001, 2_100_030),
        "actual per-reference L/V arithmetic"
    );
    let placement = Placement::new(receipt.owner.clone());
    let keeper = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3600),
        },
    );
    let round = keeper
        .round()
        .await
        .expect("normal keeper resumes the recorded owner");
    assert!(
        round.replicas == 1 && placement.owned().contains(Shard::of(receipt.run.as_uuid())),
        "real owned shard and generation"
    );
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let target = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
    let frontier = match platform
        .api
        .executions
        .collection_frontier(receipt.tenant, receipt.run, &target)
        .await
    {
        Ok(Some(f)) => f,
        _ => panic!("actual current collection frontier"),
    };
    assert!(
        frontier.phase == CollectionPhase::Open
            && frontier.run.version == receipt.head as u64
            && frontier.run.effective_output_bytes == 4_200_031
            && frontier.run.stop_reason.is_none()
            && frontier.run.stop_code.is_none()
            && frontier.item_count == 2
            && frontier.next_admission == 1
            && frontier.completed == 0
            && frontier.completed + 1 < frontier.item_count
            && frontier.max_concurrency == 1
            && frontier.active.len() == 1,
        "genuine open NONFINAL Return frontier before over-budget guard"
    );
    assert!(
        frontier.scope_id.as_uuid() == receipt.scope,
        "frontier identifies the exact original collection scope"
    );
    let member = &frontier.active[0];
    assert!(member.entry.node().as_str()=="work" && member.entry.path().segments().len()==1 && member.entry.path().segments()[0].scope_node().as_str()=="c10" && member.entry.path().segments()[0].iteration_index()==0 && member.steps.get(&member.entry).is_some_and(|step|step.status()==sagat_domain::execution::StepStatus::Succeeded),"frontier contains the exact successful original local work");
    assert!(
        member.index == 0
            && member.first_failure().is_none()
            && member
                .pending_return(&frontier.run.definition)
                .is_some_and(|r| r.node().as_str() == "returned")
            && frontier.data.output(&member.entry) == Some(b"1".as_slice()),
        "exact pending Return and actual original local output"
    );
    drop(frontier);
    let first = scheduler
        .advance_collection(receipt.tenant, receipt.run, &target)
        .await;
    assert!(
        matches!(first, Ok(CollectionAdvance::DrainStarted)),
        "normal current dispatcher drains before checkpoint/admission"
    );
    let after = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    assert!(
        after["run"]["status"] == "CANCELLING"
            && after["run"]["stop_reason"] == "FAILED"
            && after["run"]["stop_code"] == "RUN_DATA_LIMIT"
            && after["run"]["trace_id"] == before["run"]["trace_id"]
            && after["run"]["run_version"] == receipt.head + 2
            && after["telemetry"]["head_seq"] == receipt.head + 2
            && after["scope"]["lifecycle"] == "DRAINING"
            && after["scope"]["drain_kind"] == "RUN_DATA_LIMIT"
            && after["scope"]["drain_failure_code"] == "RUN_DATA_LIMIT"
            && after["scope"]["drain_iteration"].is_null()
            && after["scope"]["drain_sequence"] == receipt.head + 2,
        "one truthful atomic first stop and native drain"
    );
    for key in [
        "members",
        "steps",
        "checkpoints",
        "items",
        "payloads",
        "receipts",
        "usage",
        "iteration_usage",
        "sagas",
        "saga_entries",
    ] {
        assert!(
            before[key] == after[key],
            "first guard preserves source, active reservations, outputs and original receipts"
        );
    }
    assert!(
        nonfinal_step(&after, "returned") == nonfinal_step(&before, "returned")
            && nonfinal_step(&after, "work") == nonfinal_step(&before, "work"),
        "first drain leaves Return PENDING and known work untouched"
    );
    let suffix = after["events"]
        .as_array()
        .expect("journal rows")
        .iter()
        .filter(|e| e["seq"].as_i64().is_some_and(|seq| seq > receipt.head))
        .collect::<Vec<_>>();
    assert!(
        suffix.len() == 2
            && suffix[0]["event_type"] == "run.stop_requested"
            && suffix[0]["detail"]["reason"] == "FAILED"
            && suffix[0]["detail"]["code"] == "RUN_DATA_LIMIT"
            && suffix[1]["event_type"] == "collection.drain_started"
            && suffix[1]["node"] == "c10"
            && suffix[1]["detail"]["reason"] == "run_data_limit",
        "exact stop then drain journal suffix"
    );
    let owner_id = before["steps"]
        .as_array()
        .expect("original steps")
        .iter()
        .find(|step| step["node"] == "c10" && step["instance_path_nodes"] == json!([]))
        .expect("original root collection owner")["step_id"]
        .clone();
    let stamp = suffix[0]["occurred_at"].clone();
    assert!(stamp.is_string(), "native event timestamp is present");
    let envelope = |seq: i64,
                    node: Value,
                    instance: Value,
                    nodes: Value,
                    indices: Value,
                    kind: &str,
                    detail: Value| json!({"seq":seq,"node":node,"actor":"system:engine","detail":detail,"run_id":receipt.run.to_string(),"attempt":null,"trace_id":before["run"]["trace_id"],"tenant_id":receipt.tenant.to_string(),"event_type":kind,"instance_id":instance,"occurred_at":stamp,"payload_digest":null,"instance_path_nodes":nodes,"instance_path_indices":indices});
    assert!(
        *suffix[0]
            == envelope(
                receipt.head + 1,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                "run.stop_requested",
                json!({"code":"RUN_DATA_LIMIT","reason":"FAILED"})
            )
            && *suffix[1]
                == envelope(
                    receipt.head + 2,
                    json!("c10"),
                    owner_id,
                    json!([]),
                    json!([]),
                    "collection.drain_started",
                    json!({"reason":"run_data_limit"})
                ),
        "exact ordered event envelopes retain root/owner identity, original actor/trace and atomic timestamp"
    );
    let mut original_scope = before["scope"].as_object().expect("original scope").clone();
    let mut drained_scope = after["scope"].as_object().expect("draining scope").clone();
    for key in [
        "lifecycle",
        "drain_kind",
        "drain_iteration",
        "drain_failure_code",
        "drain_sequence",
        "drain_outbox_id",
        "updated_at",
    ] {
        original_scope.remove(key);
        drained_scope.remove(key);
    }
    assert!(
        original_scope == drained_scope,
        "first guard preserves all scope counters and source metadata"
    );
    let old_events = before["events"].as_array().expect("original journal");
    let events = after["events"].as_array().expect("first-drain journal");
    assert!(
        events.get(..old_events.len()) == Some(old_events.as_slice()),
        "first guard preserves the full original journal prefix"
    );
    let old_outbox = before["outbox"].as_array().expect("original outbox rows");
    let new_outbox = after["outbox"].as_array().expect("current outbox rows");
    assert!(
        old_outbox.iter().all(|o| new_outbox.contains(o))
            && new_outbox.len() == old_outbox.len() + 1,
        "one outbox append preserves the full original outbox"
    );
    let hint = new_outbox
        .iter()
        .find(|o| o["id"] == after["scope"]["drain_outbox_id"])
        .expect("actual native drain outbox identity");
    assert!(
        hint["event_type"] == "collection.drain.started"
            && hint["routing_key"] == "c10"
            && hint["payload"]["reason"] == "FAILED"
            && hint["payload"]["tenantId"] == receipt.tenant.to_string()
            && hint["payload"]["namespaceId"] == receipt.namespace.to_string()
            && hint["payload"]["runId"] == receipt.run.to_string()
            && hint["payload"]["scopeId"] == receipt.scope.to_string()
            && hint["payload"]["traceId"] == before["run"]["trace_id"],
        "actual public FAILED hint has exact scope, tenant, namespace, run and trace"
    );
    let old_audits = before["audits"].as_array().expect("old audits");
    let all_audits = after["audits"].as_array().expect("first-drain audits");
    let appended = all_audits
        .iter()
        .filter(|a| !old_audits.contains(a))
        .collect::<Vec<_>>();
    assert!(
        old_audits.iter().all(|a| all_audits.contains(a))
            && all_audits.len() == old_audits.len() + 1
            && appended.len() == 1,
        "exactly one audit append preserves all old audits"
    );
    let audit = appended[0];
    assert!(
        audit["action"] == "run.data_limit_exceeded"
            && audit["actor"] == "system:engine"
            && audit["resource_type"] == "run"
            && audit["tenant_id"] == receipt.tenant.to_string()
            && audit["resource_id"] == receipt.run.to_string()
            && audit["reason"] == "existing_outputs"
            && audit["trace_id"] == before["run"]["trace_id"],
        "exact truthful existing_outputs audit identity"
    );
    assert!(
        nonfinal_source(
            &platform.api.db.pool,
            receipt.tenant.as_uuid(),
            receipt.run.as_uuid(),
            receipt.cutoff
        )
        .await
            == receipt.source,
        "first native guard preserves all old immutable source"
    );
    let mut worker = platform.connect("nonfinal-return-worker", TENANT).await;
    worker
        .register_version(&["line.reserve", "line.release"], "1.5")
        .await;
    let mut actions = 0usize;
    let comp = loop {
        actions += 1;
        assert!(
            actions <= 32,
            "bounded current drain and compensation transitions"
        );
        assert!(
            scheduler
                .advance_collection(receipt.tenant, receipt.run, &target)
                .await
                .is_ok(),
            "normal member drain or saga planning"
        );
        let mut tasks = worker.poll(1).await;
        if let Some(task) = tasks.pop() {
            assert!(tasks.is_empty(), "one compensation obligation");
            break task;
        }
    };
    let original = comp
        .target_instance
        .as_ref()
        .expect("original compensation target");
    let input: Value = serde_json::from_str(&comp.input_json)
        .unwrap_or_else(|_| panic!("actual compensation input JSON"));
    assert!(
        comp.activity_name == "line.release"
            && comp.task_kind == "COMPENSATION"
            && comp.target_node == "work"
            && original.definition_node_id == "work"
            && original.instance_path.len() == 1
            && original.instance_path[0].scope_node_id == "c10"
            && original.instance_path[0].iteration_index == 0
            && comp.target_idempotency_key == receipt.target_key
            && input.pointer("/steps/work/output") == Some(&serde_json::json!(1)),
        "real original target/key and sufficient old output in compensation input"
    );
    let waiting = scheduler
        .advance_collection(receipt.tenant, receipt.run, &target)
        .await;
    assert!(
        matches!(waiting, Ok(CollectionAdvance::Waiting)),
        "unfinished real compensation prevents premature finalization"
    );
    let held = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    assert!(
        held["scope"]["lifecycle"] == "DRAINING" && held["run"]["finished_at"].is_null(),
        "scope and Run remain nonterminal with leased compensation"
    );
    let request = worker.req(Worker::complete_message(&comp, r#"{"released":true}"#));
    let ack = match worker.client.complete_step(request).await {
        Ok(r) => r.into_inner(),
        Err(_) => panic!("real compensation completion succeeds"),
    };
    assert!(
        ack.step_status == "SUCCEEDED" && !ack.already_applied,
        "genuine compensation success"
    );
    loop {
        actions += 1;
        assert!(actions <= 32, "bounded normal finalization");
        assert!(
            scheduler
                .advance_collection(receipt.tenant, receipt.run, &target)
                .await
                .is_ok(),
            "normal saga and collection finalization"
        );
        let state = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
        if state["run"]["status"] == "COMPENSATED" {
            break;
        }
    }
    let final_state = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    assert!(
        final_state["run"]["status"] == "COMPENSATED"
            && final_state["run"]["stop_reason"] == "FAILED"
            && final_state["run"]["stop_code"] == "RUN_DATA_LIMIT"
            && final_state["run"]["result_digest"].is_null()
            && final_state["scope"]["lifecycle"] == "FAILED"
            && final_state["scope"]["result_digest"].is_null()
            && final_state["scope"]["next_admission_index"] == 1
            && final_state["scope"]["admitted_count"] == 1
            && final_state["scope"]["completed_count"] == 1
            && final_state["scope"]["active_count"] == 0
            && final_state["iteration_usage"]["active_iterations"] == 0,
        "normal failed drain settles exactly the admitted member, without admitting item1"
    );
    assert!(
        nonfinal_step(&final_state, "returned")["status"] == "CANCELLED"
            && nonfinal_step(&final_state, "returned")["output_digest"].is_null()
            && nonfinal_step(&final_state, "work") == nonfinal_step(&before, "work")
            && nonfinal_step(&final_state, "work~c")["status"] == "SUCCEEDED"
            && nonfinal_step(&final_state, "work~c")["output_digest"].is_null(),
        "Return cancelled, old forward source intact and actual compensation settled without output"
    );
    let (checkpoints,item1,ends,saga_done):(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM iteration_checkpoints WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3),(SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND iteration_index=1),(SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='done'),(SELECT count(*) FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 AND owner_node='c10' AND phase='DONE' AND outcome='COMPENSATED')")
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).bind(receipt.scope).fetch_one(&platform.api.db.pool).await.expect("honest nonfinal terminal identities");
    assert!(
        (checkpoints, item1, ends, saga_done) == (0, 0, 0, 1)
            && before["payloads"] == final_state["payloads"]
            && nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
                == (2_100_001, 2_100_030)
            && nonfinal_source(
                &platform.api.db.pool,
                receipt.tenant.as_uuid(),
                receipt.run.as_uuid(),
                receipt.cutoff
            )
            .await
                == receipt.source,
        "no backfill, new source, checkpoint, End or payload; exact old L/V/source retained"
    );
    let members = final_state["members"].as_array().expect("settled members");
    assert!(
        members.len() == 1
            && members[0]["iteration_index"] == 0
            && members[0]["lifecycle"] == "DRAINED"
            && members[0]["slot_held"] == false
            && members[0]["remaining_reservations"] == 0
            && members[0]["checkpoint_sequence"].is_null()
            && members[0]["result_digest"].is_null()
            && final_state["usage"]["reserved_instances"] == 0,
        "sole member drained without checkpoint and every reservation released"
    );
    let initial_steps = before["steps"].as_array().expect("original steps");
    let final_steps = final_state["steps"].as_array().expect("terminal steps");
    assert!(
        initial_steps.iter().all(|old| final_steps
            .iter()
            .any(|new| new["step_id"] == old["step_id"]))
            && final_steps.len() == initial_steps.len() + 1,
        "exactly one derived step identity preserves all original identities"
    );
    let derived = final_steps
        .iter()
        .filter(|new| {
            !initial_steps
                .iter()
                .any(|old| old["step_id"] == new["step_id"])
        })
        .collect::<Vec<_>>();
    let own = comp
        .instance
        .as_ref()
        .expect("actual compensation instance");
    assert!(
        derived.len() == 1
            && *derived[0] == *nonfinal_step(&final_state, "work~c")
            && derived[0]["step_id"] == comp.step_id
            && derived[0]["kind"] == "COMPENSATION"
            && comp.run_id == receipt.run.to_string()
            && own.definition_node_id == "work~c"
            && own.instance_path.len() == 1
            && own.instance_path[0].scope_node_id == "c10"
            && own.instance_path[0].iteration_index == 0,
        "leased compensation is the only new execution identity"
    );
    let entries = final_state["saga_entries"]
        .as_array()
        .expect("native saga entries");
    assert!(
        entries.len() == 1
            && entries[0]["owner_node"] == "c10"
            && entries[0]["owner_path_nodes"] == json!([])
            && entries[0]["owner_path_indices"] == json!([])
            && entries[0]["iteration_index"] == 0
            && entries[0]["node"] == "work"
            && entries[0]["kind"] == "COMPENSATE",
        "exactly one original work compensation obligation"
    );
    let owner = final_steps
        .iter()
        .find(|s| s["node"] == "c10" && s["instance_path_nodes"] == json!([]))
        .expect("terminal root owner");
    assert!(
        owner["status"] == "FAILED"
            && owner["failure_code"] == "RUN_DATA_LIMIT"
            && owner["output_digest"].is_null(),
        "failed root owner never materializes a legacy output"
    );
    let final_events = final_state["events"].as_array().expect("terminal journal");
    assert!(
        final_events.get(..old_events.len()) == Some(old_events.as_slice())
            && final_state["checkpoints"] == before["checkpoints"],
        "entire old journal and checkpoint inventories stay exact"
    );
    let finish_seq = final_state["scope"]["finish_sequence"]
        .as_i64()
        .expect("native collection finish sequence");
    let finishes = final_events
        .iter()
        .filter(|e| e["event_type"] == "collection.finished" && e["node"] == "c10")
        .collect::<Vec<_>>();
    let run_finishes = final_events
        .iter()
        .filter(|e| e["event_type"] == "run.finished")
        .collect::<Vec<_>>();
    assert!(
        finishes.len() == 1
            && finishes[0]["seq"] == finish_seq
            && finishes[0]["detail"] == json!({"outcome":"failed","code":"RUN_DATA_LIMIT"})
            && run_finishes.len() == 1
            && run_finishes[0]["seq"] == finish_seq + 1
            && run_finishes[0]["detail"]["status"] == "COMPENSATED"
            && final_state["run"]["run_version"] == finish_seq + 1
            && final_events
                .last()
                .is_some_and(|e| e["seq"] == finish_seq + 1)
            && final_state["telemetry"]["head_seq"] == finish_seq + 1,
        "exact native finish pair and final telemetry head"
    );
    let terminal_outbox = final_state["outbox"].as_array().expect("terminal outbox");
    let collection_hints = terminal_outbox
        .iter()
        .filter(|o| o["event_type"] == "collection.finished" && o["routing_key"] == "c10")
        .collect::<Vec<_>>();
    let run_hints = terminal_outbox
        .iter()
        .filter(|o| o["event_type"] == "run.finished")
        .collect::<Vec<_>>();
    assert!(
        collection_hints.len() == 1
            && collection_hints[0]["id"] == final_state["scope"]["finish_outbox_id"]
            && run_hints.len() == 1,
        "exact native finish outbox identities"
    );
    for (hint, status) in [
        (collection_hints[0], "FAILED"),
        (run_hints[0], "COMPENSATED"),
    ] {
        assert!(
            hint["tenant_id"] == receipt.tenant.to_string()
                && hint["aggregate_id"] == receipt.run.to_string()
                && hint["payload"]["tenantId"] == receipt.tenant.to_string()
                && hint["payload"]["namespaceId"] == receipt.namespace.to_string()
                && hint["payload"]["runId"] == receipt.run.to_string()
                && hint["payload"]["traceId"] == before["run"]["trace_id"]
                && hint["payload"]["status"] == status,
            "finish hints preserve exact original envelope and truthful terminal status"
        );
    }
    assert!(
        collection_hints[0]["payload"]["scopeId"] == receipt.scope.to_string(),
        "failed collection hint matches original scope"
    );
    let terminal_projection =
        projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await;
    let fresh = PgRunStore::new(
        platform.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    assert!(
        fresh.verify_run(receipt.tenant, receipt.run).await.is_ok(),
        "fresh store safely verifies mixed old/current compensated history"
    );
    let terminal = scheduler
        .advance_collection(receipt.tenant, receipt.run, &target)
        .await;
    assert!(
        matches!(terminal, Ok(CollectionAdvance::AlreadyFinished)),
        "normal terminal advance is already finished"
    );
    assert!(
        nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await == final_state,
        "terminal replay and fresh verification have zero durable effects"
    );
    assert!(
        projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await
            == terminal_projection,
        "fresh verification and terminal advance preserve the full original projection and certifications"
    );
    drop(fresh);
    drop(worker);
    drop(scheduler);
    keeper.leave().await;
    drop(keeper);
    let role_pool = platform.api.api_pool.clone();
    let owner_pool = platform.api.db.pool.clone();
    drop(platform);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), role_pool.close())
            .await
            .is_ok(),
        "current role pool closes"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), owner_pool.close())
            .await
            .is_ok(),
        "current owner pool closes"
    );
}

// Closed scenario-specific handoffs; worker capabilities never cross these processes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AdmissionFixture {
    OverLimit,
    EqualLimit,
    FailedMember,
}

impl AdmissionFixture {
    fn format(self) -> &'static str {
        match self {
            Self::OverLimit => "legacy-free-admission-frontier-v1",
            Self::EqualLimit => "legacy-free-admission-equal-limit-v1",
            Self::FailedMember => "legacy-failed-member-frontier-v1",
        }
    }
    fn case(self) -> &'static str {
        match self {
            Self::OverLimit => "free-admission",
            Self::EqualLimit => "free-admission-equal-limit",
            Self::FailedMember => "failed-member",
        }
    }
}

struct FreeAdmissionReceipt {
    database_name: String,
    database_url: String,
    runtime_url: String,
    tenant: TenantId,
    other_tenant: TenantId,
    namespace: Uuid,
    run: RunId,
    version: Uuid,
    scope: Uuid,
    owner: String,
    head: i64,
    cutoff: i64,
    projection: String,
    source: String,
}

impl FreeAdmissionReceipt {
    fn parse(bytes: &[u8], scenario: AdmissionFixture) -> Self {
        let value: Value = serde_json::from_slice(bytes)
            .unwrap_or_else(|_| panic!("invalid private free-admission handoff JSON"));
        let mut fields = value
            .as_object()
            .expect("free-admission handoff object")
            .clone();
        assert!(fields.len() == 18, "closed eighteen-field scenario handoff");
        let mut take = |name: &str| {
            fields
                .remove(name)
                .expect("required settled-work handoff field")
        };
        let text = |value: Value| {
            value
                .as_str()
                .expect("settled-work string field")
                .to_owned()
        };
        assert!(
            text(take("format")) == scenario.format()
                && text(take("source_commit")) == PINNED_LEGACY_COMMIT
                && text(take("case")) == scenario.case(),
            "exact private free-admission format and immutable source"
        );
        let role = text(take("role"));
        assert!(
            role == if crate::support::as_runtime_role() {
                "runtime"
            } else {
                "owner"
            },
            "handoff and consumer roles match"
        );
        let database_name = text(take("database_name"));
        assert!(
            database_name.starts_with("t_")
                && database_name.len() == 34
                && database_name[2..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "exact owned database name"
        );
        let receipt = Self {
            database_name,
            database_url: text(take("database_url")),
            runtime_url: text(take("runtime_database_url")),
            tenant: text(take("tenant")).parse().expect("old tenant id"),
            other_tenant: text(take("other_tenant"))
                .parse()
                .expect("old other tenant id"),
            namespace: text(take("namespace")).parse().expect("old namespace id"),
            run: text(take("run")).parse().expect("old run id"),
            version: text(take("workflow_version"))
                .parse()
                .expect("old pinned version"),
            scope: text(take("scope_id")).parse().expect("old target scope"),
            owner: text(take("scheduler_owner")),
            head: take("handoff_run_version")
                .as_i64()
                .expect("old journal head"),
            cutoff: take("closed_source_cutoff")
                .as_i64()
                .expect("old closed prefix cutoff"),
            projection: text(take("producer_witness_sha256")),
            source: text(take("closed_source_sha256")),
        };
        assert!(
            fields.is_empty()
                && receipt.owner == format!("free-admission-{role}")
                && receipt.head
                    == receipt.cutoff
                        + if scenario == AdmissionFixture::FailedMember {
                            6
                        } else {
                            1
                        }
                && receipt.cutoff > 0,
            "complete settled-work identity"
        );
        for hash in [&receipt.projection, &receipt.source] {
            assert!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "canonical source SHA256"
            );
        }
        receipt
    }
}

async fn free_admission_source(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    cutoff: i64,
) -> String {
    admission_source(pool, tenant, run, cutoff, false).await
}

async fn admission_source(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    cutoff: i64,
    failed_member: bool,
) -> String {
    let closed: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
        .bind(tenant)
        .bind(run)
        .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .expect("immutable old closed source");
    let mut target:Value=sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'run',to_jsonb(r)-ARRAY['status','run_version','stop_reason','stop_code','updated_at','finished_at'],
      'version',to_jsonb(version_row),
      'scope',to_jsonb(s)-ARRAY['lifecycle','drain_kind','drain_iteration','drain_failure_code','drain_sequence','drain_outbox_id','finish_sequence','finish_outbox_id','updated_at'],
      'owner',to_jsonb(o)-ARRAY['status','waiting_kind','failure_code','updated_at'],
      'snapshot',to_jsonb(snapshot_row),'snapshot_json',convert_from(snapshot_row.content,'UTF8')::jsonb,
      'captured',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) FROM collection_scope_items i WHERE i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id),
      'item_payloads',(SELECT jsonb_agg(to_jsonb(p) ORDER BY i.iteration_index) FROM collection_scope_items i JOIN payloads p ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest WHERE i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id),
      'entry_event',(SELECT to_jsonb(e) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq=$3),
      'entry_matches',(SELECT e.instance_id=o.step_id AND e.instance_path_nodes=s.owner_path_nodes AND e.instance_path_indices=s.owner_path_indices AND e.node=s.owner_node AND e.actor='system:engine' AND e.trace_id=r.trace_id AND e.attempt IS NULL AND e.payload_digest IS NULL AND e.occurred_at=s.created_at AND e.detail=jsonb_build_object('item_count',s.item_count,'max_items',s.max_items,'max_concurrency',s.max_concurrency,'snapshot_digest',encode(s.snapshot_digest,'hex'),'source_blob',s.source_blob_id::text) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq=$3))
      FROM runs r JOIN workflow_versions version_row ON version_row.tenant_id=r.tenant_id AND version_row.id=r.workflow_version_id
      JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND cardinality(s.owner_path_nodes)=0
      JOIN steps o ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node AND cardinality(o.instance_path_nodes)=0
      JOIN payloads snapshot_row ON snapshot_row.tenant_id=s.tenant_id AND snapshot_row.digest=s.snapshot_digest
      WHERE r.tenant_id=$1 AND r.id=$2"#).bind(tenant).bind(run).bind(cutoff+1)
        .fetch_one(pool).await.expect("real immutable free-admission target source");
    assert!(
        target["run"].is_object()
            && target["run"]["id"] == run.to_string()
            && target["entry_matches"] == true
            && target["version"].is_object()
            && target["version"]["id"].is_string()
            && target["scope"]["owner_node"] == "c10"
            && target["snapshot_json"] == json!([1, 2])
            && target["captured"].as_array().is_some_and(|a| a.len() == 2)
            && target["item_payloads"]
                .as_array()
                .is_some_and(|a| a.len() == 2)
            && target["entry_event"]["seq"] == cutoff + 1
            && target["entry_event"]["event_type"] == "collection.entered"
            && target["entry_event"]["node"] == "c10",
        "nonvacuous pinned definition, captured source, payloads and original entry"
    );
    if failed_member {
        let scope = target["scope"]
            .as_object_mut()
            .expect("actual target scope");
        scope.remove("active_count");
        scope.remove("completed_count");
        let failure: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'step',to_jsonb(w),
          'member',(SELECT to_jsonb(m)-ARRAY['lifecycle','slot_held','remaining_reservations','drained_sequence','drain_outbox_id','updated_at'] FROM collection_iterations m WHERE m.tenant_id=w.tenant_id AND m.run_id=w.run_id AND m.entry_instance_id=w.step_id),
          'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=w.tenant_id AND e.run_id=w.run_id AND e.seq BETWEEN $3+2 AND $3+6),
          'receipts',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.scope,c.idempotency_key) FROM command_dedupe c WHERE c.tenant_id=w.tenant_id AND c.scope='step:'||w.run_id::text||':scoped:3:c10:0;10:legacyWork:'||substring(encode(w.reported_lease_token_hash,'hex') from 1 for 16)),
          'reported',octet_length(w.reported_lease_token_hash)=32 AND w.lease_token_hash IS NULL AND w.leased_by IS NULL AND w.lease_expires_at IS NULL)
          FROM steps w WHERE w.tenant_id=$1 AND w.run_id=$2 AND w.node='legacyWork' AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[]"#)
            .bind(tenant).bind(run).bind(cutoff).fetch_one(pool).await.expect("real failed-member source and original report receipt");
        let receipts = failure["receipts"]
            .as_array()
            .expect("original failure receipt");
        assert!(
            failure["step"]["status"] == "FAILED"
                && failure["step"]["routed"] == true
                && failure["step"]["failure_code"] == "Q5_MEMBER_FAILURE"
                && failure["step"]["output_digest"].is_null()
                && failure["reported"] == true
                && failure["member"]["entry_instance_id"] == failure["step"]["step_id"]
                && failure["member"]["entry_path_nodes"] == json!(["c10"])
                && failure["member"]["entry_path_indices"] == json!([0])
                && failure["member"]["entry_node"] == "legacyWork"
                && failure["member"]["admitted_sequence"] == cutoff + 2
                && receipts.len() == 1
                && receipts[0]["resource_id"] == format!("FAILED|WAITING|{}", cutoff + 6),
            "actual terminal failed work and canonical original receipt, no active lease or output"
        );
        let events = failure["events"]
            .as_array()
            .expect("original old admission/claim/failure suffix");
        assert!(
            events.len() == 5,
            "exact old five-event member suffix after entry"
        );
        for (i, kind) in [
            "collection.iteration_admitted",
            "step.released",
            "step.leased",
            "step.failed",
            "step.routed",
        ]
        .iter()
        .enumerate()
        {
            assert!(
                events[i]["seq"] == cutoff + 2 + i as i64
                    && events[i]["event_type"] == *kind
                    && events[i]["tenant_id"] == tenant.to_string()
                    && events[i]["run_id"] == run.to_string()
                    && events[i]["trace_id"] == target["run"]["trace_id"],
                "exact ordered old failure source event identity"
            );
            if i > 0 {
                assert!(
                    events[i]["instance_id"] == failure["step"]["step_id"]
                        && events[i]["node"] == "legacyWork"
                        && events[i]["instance_path_nodes"] == json!(["c10"])
                        && events[i]["instance_path_indices"] == json!([0]),
                    "failure suffix retains exact original scoped entry"
                );
            }
        }
        assert!(
            events[3]["detail"]["code"] == "Q5_MEMBER_FAILURE"
                && events[4]["detail"]["activated"] == json!([]),
            "known nonretryable failure routes no successor"
        );
        target["failed_member"] = failure;
    }
    nonfinal_sha(&json!({"closed":closed,"target":target}))
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the separate immutable-old free-admission producer; see fixture README"]
async fn current_frontier_drains_frozen_old_free_admission_before_first_member() {
    free_admission_current(AdmissionFixture::OverLimit).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires separate immutable-old equal-limit producer; see fixture README"]
async fn current_frontier_admits_frozen_old_free_admission_at_exact_limit() {
    free_admission_current(AdmissionFixture::EqualLimit).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires separate immutable-old failed-member producer; see fixture README"]
async fn current_frontier_preserves_frozen_old_member_failure_over_excess() {
    free_admission_current(AdmissionFixture::FailedMember).await;
}

async fn free_admission_current(scenario: AdmissionFixture) {
    let equal_limit = scenario == AdmissionFixture::EqualLimit;
    let failed_member = scenario == AdmissionFixture::FailedMember;
    use crate::support::{
        api::{Options, start_on_db},
        attach_existing_db,
        worker::Platform,
    };
    use sagat_application::scope_frontier::ScopeFrontierStore as _;
    use sagat_domain::execution::CollectionPhase;
    use sagat_scheduler::CollectionAdvance;
    assert!(
        std::env::var("SAGAT_Q5_OWNED_POSTGRES").as_deref() == Ok("1"),
        "only the root-owned repository PostgreSQL fixture"
    );
    let path = std::path::PathBuf::from(
        std::env::var_os("SAGAT_Q5_PHASE_RECEIPT").expect("private settled-work receipt"),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::metadata(&path)
                .expect("private receipt metadata")
                .permissions()
                .mode()
                & 0o777
                == 0o600,
            "private receipt permissions"
        );
    }
    let receipt = FreeAdmissionReceipt::parse(
        &std::fs::read(&path).expect("read private settled-work handoff"),
        scenario,
    );
    let admin = std::env::var("SAGAT_TEST_DATABASE_URL").expect("owned fixture admin URL");
    let db = attach_existing_db(&admin, &receipt.database_name).await;
    assert!(
        receipt.database_url == db.url
            && receipt.runtime_url
                == db.url_for(
                    crate::support::RUNTIME_ROLE,
                    crate::support::RUNTIME_ROLE_PASSWORD
                ),
        "receipt URLs are references to this exact owned fixture, never arbitrary targets"
    );
    for migrated in [false, true] {
        if migrated {
            migrate(&db.pool)
                .await
                .expect("current migration of genuine old data");
        }
        assert!(
            projection_witness(&db.pool, receipt.tenant, &[receipt.run]).await
                == receipt.projection,
            "migration and handoff preserve every old projection byte"
        );
        assert!(
            admission_source(
                &db.pool,
                receipt.tenant.as_uuid(),
                receipt.run.as_uuid(),
                receipt.cutoff,
                failed_member,
            )
            .await
                == receipt.source,
            "validated old closed and target source stay exact"
        );
    }
    let api = start_on_db(
        db,
        Options {
            runtime_role: crate::support::as_runtime_role(),
            ..Options::default()
        },
    )
    .await;
    let platform = Platform {
        api,
        tenant: receipt.tenant,
        other_tenant: receipt.other_tenant,
    };
    let role: (String, String) =
        sqlx::query_as("SELECT current_user::text,current_database()::text")
            .fetch_one(&platform.api.api_pool)
            .await
            .expect("current API/store shared pool identity");
    let owner_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("current inspection owner");
    assert!(
        role.1 == receipt.database_name
            && role.0
                == if crate::support::as_runtime_role() {
                    crate::support::RUNTIME_ROLE
                } else {
                    owner_role.as_str()
                },
        "actual selected current API and execution-store role"
    );
    println!(
        "current_free_admission_role api={} store={} database={}",
        role.0, role.0, role.1
    );
    let before = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    assert!(
        before["run"]["workflow_version_id"] == receipt.version.to_string()
            && before["run"]["namespace_id"] == receipt.namespace.to_string()
            && before["scope"]["scope_id"] == receipt.scope.to_string()
            && before["run"]["run_version"] == receipt.head,
        "exact original published and scope identities"
    );
    assert!(
        before["run"]["status"] == "WAITING"
            && (if failed_member {
                before["members"].as_array().is_some_and(|m| m.len() == 1)
            } else {
                before["members"].is_null()
            })
            && before["scope"]["admitted_count"] == if failed_member { 1 } else { 0 }
            && before["scope"]["active_count"] == if failed_member { 1 } else { 0 }
            && before["usage"]["reserved_instances"] == if failed_member { 1 } else { 0 }
            && before["iteration_usage"]["active_iterations"] == if failed_member { 1 } else { 0 },
        "genuine WAITING scope has exact selected member and reservation counts"
    );
    assert!(
        nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == if equal_limit {
                (2_097_137, 2_097_167)
            } else {
                (2_100_000, 2_100_030)
            },
        "actual per-reference L/V arithmetic"
    );
    let placement = Placement::new(receipt.owner.clone());
    let keeper = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3600),
        },
    );
    let round = keeper
        .round()
        .await
        .expect("normal keeper resumes the recorded owner");
    assert!(
        round.replicas == 1 && placement.owned().contains(Shard::of(receipt.run.as_uuid())),
        "real owned shard and generation"
    );
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let target = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
    let frontier = match platform
        .api
        .executions
        .collection_frontier(receipt.tenant, receipt.run, &target)
        .await
    {
        Ok(Some(f)) => f,
        _ => panic!("actual current collection frontier"),
    };
    assert!(
        frontier.phase == CollectionPhase::Open
            && frontier.run.version == receipt.head as u64
            && frontier.run.effective_output_bytes
                == if equal_limit { 4_194_304 } else { 4_200_030 }
            && frontier.run.stop_reason.is_none()
            && frontier.run.stop_code.is_none()
            && frontier.item_count == 2
            && frontier.next_admission == if failed_member { 1 } else { 0 }
            && frontier.completed == 0
            && frontier.max_concurrency == 1
            && if failed_member {
                frontier.active.len() == 1
                    && frontier.active[0].index == 0
                    && frontier.active[0].steps.len() == 1
                    && frontier.active[0]
                        .first_failure()
                        .is_some_and(|code| code.as_str() == "Q5_MEMBER_FAILURE")
                    && frontier.active[0]
                        .pending_return(&frontier.run.definition)
                        .is_none()
            } else {
                frontier.active.is_empty()
            }
            && !frontier.run.definition.scope_compensates(&target),
        "genuine selected legacy frontier before its first current transition"
    );
    assert!(
        frontier.scope_id.as_uuid() == receipt.scope,
        "frontier identifies the exact original collection scope"
    );
    drop(frontier);
    if failed_member {
        assert_failed_member_flow(&platform, &receipt, &scheduler, &target, &before).await;
    } else if equal_limit {
        let closed_before: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
            .bind(receipt.tenant.as_uuid())
            .bind(receipt.run.as_uuid())
            .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
            .bind(receipt.cutoff)
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("exact closed old source before equal-limit admission");
        assert!(
            matches!(
                scheduler
                    .advance_collection(receipt.tenant, receipt.run, &target)
                    .await,
                Ok(CollectionAdvance::Admitted { index: 0 })
            ),
            "normal current frontier admits index0 at exactly4MiB"
        );
        assert_equal_limit_admission(&platform, &receipt, &before, &closed_before).await;
    } else {
        let first = scheduler
            .advance_collection(receipt.tenant, receipt.run, &target)
            .await;
        assert!(
            matches!(first, Ok(CollectionAdvance::DrainStarted)),
            "normal current dispatcher drains before first admission"
        );
        let after = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
        assert!(
            after["run"]["status"] == "CANCELLING"
                && after["run"]["stop_reason"] == "FAILED"
                && after["run"]["stop_code"] == "RUN_DATA_LIMIT"
                && after["run"]["trace_id"] == before["run"]["trace_id"]
                && after["run"]["run_version"] == receipt.head + 2
                && after["telemetry"]["head_seq"] == receipt.head + 2
                && after["scope"]["lifecycle"] == "DRAINING"
                && after["scope"]["drain_kind"] == "RUN_DATA_LIMIT"
                && after["scope"]["drain_failure_code"] == "RUN_DATA_LIMIT"
                && after["scope"]["drain_iteration"].is_null()
                && after["scope"]["drain_sequence"] == receipt.head + 2,
            "one truthful atomic first stop and native drain"
        );
        for key in [
            "members",
            "steps",
            "checkpoints",
            "items",
            "payloads",
            "receipts",
            "usage",
            "iteration_usage",
            "sagas",
            "saga_entries",
        ] {
            assert!(
                before[key] == after[key],
                "first guard preserves source, active reservations, outputs and original receipts"
            );
        }
        let suffix = after["events"]
            .as_array()
            .expect("journal rows")
            .iter()
            .filter(|e| e["seq"].as_i64().is_some_and(|seq| seq > receipt.head))
            .collect::<Vec<_>>();
        assert!(
            suffix.len() == 2
                && suffix[0]["event_type"] == "run.stop_requested"
                && suffix[0]["detail"]["reason"] == "FAILED"
                && suffix[0]["detail"]["code"] == "RUN_DATA_LIMIT"
                && suffix[1]["event_type"] == "collection.drain_started"
                && suffix[1]["node"] == "c10"
                && suffix[1]["detail"]["reason"] == "run_data_limit",
            "exact stop then drain journal suffix"
        );
        let owner_id = before["steps"]
            .as_array()
            .expect("original steps")
            .iter()
            .find(|step| step["node"] == "c10" && step["instance_path_nodes"] == json!([]))
            .expect("original root collection owner")["step_id"]
            .clone();
        let stamp = suffix[0]["occurred_at"].clone();
        assert!(stamp.is_string(), "native event timestamp is present");
        let envelope = |seq: i64,
                        node: Value,
                        instance: Value,
                        nodes: Value,
                        indices: Value,
                        kind: &str,
                        detail: Value| json!({"seq":seq,"node":node,"actor":"system:engine","detail":detail,"run_id":receipt.run.to_string(),"attempt":null,"trace_id":before["run"]["trace_id"],"tenant_id":receipt.tenant.to_string(),"event_type":kind,"instance_id":instance,"occurred_at":stamp,"payload_digest":null,"instance_path_nodes":nodes,"instance_path_indices":indices});
        assert!(
            *suffix[0]
                == envelope(
                    receipt.head + 1,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    "run.stop_requested",
                    json!({"code":"RUN_DATA_LIMIT","reason":"FAILED"})
                )
                && *suffix[1]
                    == envelope(
                        receipt.head + 2,
                        json!("c10"),
                        owner_id,
                        json!([]),
                        json!([]),
                        "collection.drain_started",
                        json!({"reason":"run_data_limit"})
                    ),
            "exact ordered event envelopes retain root/owner identity, original actor/trace and atomic timestamp"
        );
        let mut original_scope = before["scope"].as_object().expect("original scope").clone();
        let mut drained_scope = after["scope"].as_object().expect("draining scope").clone();
        for key in [
            "lifecycle",
            "drain_kind",
            "drain_iteration",
            "drain_failure_code",
            "drain_sequence",
            "drain_outbox_id",
            "updated_at",
        ] {
            original_scope.remove(key);
            drained_scope.remove(key);
        }
        assert!(
            original_scope == drained_scope,
            "first guard preserves all scope counters and source metadata"
        );
        let old_events = before["events"].as_array().expect("original journal");
        let events = after["events"].as_array().expect("first-drain journal");
        assert!(
            events.get(..old_events.len()) == Some(old_events.as_slice()),
            "first guard preserves the full original journal prefix"
        );
        let old_outbox = before["outbox"].as_array().expect("original outbox rows");
        let new_outbox = after["outbox"].as_array().expect("current outbox rows");
        assert!(
            old_outbox.iter().all(|o| new_outbox.contains(o))
                && new_outbox.len() == old_outbox.len() + 1,
            "one outbox append preserves the full original outbox"
        );
        let hint = new_outbox
            .iter()
            .find(|o| o["id"] == after["scope"]["drain_outbox_id"])
            .expect("actual native drain outbox identity");
        assert!(
            hint["event_type"] == "collection.drain.started"
                && hint["routing_key"] == "c10"
                && hint["payload"]["reason"] == "FAILED"
                && hint["payload"]["tenantId"] == receipt.tenant.to_string()
                && hint["payload"]["namespaceId"] == receipt.namespace.to_string()
                && hint["payload"]["runId"] == receipt.run.to_string()
                && hint["payload"]["scopeId"] == receipt.scope.to_string()
                && hint["payload"]["traceId"] == before["run"]["trace_id"],
            "actual public FAILED hint has exact scope, tenant, namespace, run and trace"
        );
        let old_audits = before["audits"].as_array().expect("old audits");
        let all_audits = after["audits"].as_array().expect("first-drain audits");
        let appended = all_audits
            .iter()
            .filter(|a| !old_audits.contains(a))
            .collect::<Vec<_>>();
        assert!(
            old_audits.iter().all(|a| all_audits.contains(a))
                && all_audits.len() == old_audits.len() + 1
                && appended.len() == 1,
            "exactly one audit append preserves all old audits"
        );
        let audit = appended[0];
        assert!(
            audit["action"] == "run.data_limit_exceeded"
                && audit["actor"] == "system:engine"
                && audit["resource_type"] == "run"
                && audit["tenant_id"] == receipt.tenant.to_string()
                && audit["resource_id"] == receipt.run.to_string()
                && audit["reason"] == "existing_outputs"
                && audit["trace_id"] == before["run"]["trace_id"],
            "exact truthful existing_outputs audit identity"
        );
        assert!(
            free_admission_source(
                &platform.api.db.pool,
                receipt.tenant.as_uuid(),
                receipt.run.as_uuid(),
                receipt.cutoff
            )
            .await
                == receipt.source,
            "first native guard preserves all old immutable source"
        );
        let finished = scheduler
            .advance_collection(receipt.tenant, receipt.run, &target)
            .await;
        assert!(
            matches!(finished, Ok(CollectionAdvance::DrainFinished)),
            "one normal no-member advance finishes FAILED"
        );
        let final_state = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
        assert!(
            final_state["run"]["status"] == "FAILED"
                && final_state["run"]["stop_reason"] == "FAILED"
                && final_state["run"]["stop_code"] == "RUN_DATA_LIMIT"
                && final_state["run"]["result_digest"].is_null()
                && final_state["run"]["finished_at"].is_string()
                && final_state["run"]["run_version"] == receipt.head + 4
                && final_state["telemetry"]["head_seq"] == receipt.head + 4,
            "exact failed Run and final native head"
        );
        assert!(
            final_state["scope"]["lifecycle"] == "FAILED"
                && final_state["scope"]["result_digest"].is_null()
                && final_state["scope"]["finish_sequence"] == receipt.head + 3,
            "failed scope never materializes a result"
        );
        let mut expected_scope = after["scope"].as_object().expect("draining scope").clone();
        let mut finished_scope = final_state["scope"]
            .as_object()
            .expect("finished scope")
            .clone();
        for key in [
            "lifecycle",
            "finish_sequence",
            "finish_outbox_id",
            "updated_at",
        ] {
            expected_scope.remove(key);
            finished_scope.remove(key);
        }
        assert!(
            expected_scope == finished_scope,
            "finish retains original drain/source/counters without admission"
        );
        for key in [
            "members",
            "items",
            "checkpoints",
            "payloads",
            "receipts",
            "usage",
            "iteration_usage",
            "sagas",
            "saga_entries",
            "audits",
        ] {
            assert!(
                final_state[key] == after[key],
                "empty drain adds no member, task, payload, receipt, reservation, saga or audit"
            );
        }
        assert!(
            final_state["members"].is_null()
                && final_state["scope"]["next_admission_index"] == 0
                && final_state["scope"]["admitted_count"] == 0
                && final_state["scope"]["completed_count"] == 0
                && final_state["scope"]["active_count"] == 0
                && final_state["usage"]["reserved_instances"] == 0
                && final_state["iteration_usage"]["active_iterations"] == 0,
            "all progress and reservations remain exactly zero"
        );
        let initial_steps = before["steps"].as_array().expect("original steps");
        let final_steps = final_state["steps"].as_array().expect("terminal steps");
        let owner_before = initial_steps
            .iter()
            .find(|w| w["node"] == "c10" && w["instance_path_nodes"] == json!([]))
            .expect("original root owner");
        let owner_after = final_steps
            .iter()
            .find(|w| w["step_id"] == owner_before["step_id"])
            .expect("same failed owner");
        assert!(
            owner_after["status"] == "FAILED"
                && owner_after["failure_code"] == "RUN_DATA_LIMIT"
                && owner_after["waiting_kind"].is_null()
                && owner_after["output_digest"].is_null()
                && owner_after["routed"] == false,
            "failed owner is not routed and retains no output"
        );
        let mut expected_steps = initial_steps.clone();
        let expected_owner = expected_steps
            .iter_mut()
            .find(|w| w["step_id"] == owner_before["step_id"])
            .expect("expected owner");
        for key in ["status", "failure_code", "waiting_kind", "updated_at"] {
            expected_owner[key] = owner_after[key].clone();
        }
        assert!(
            &expected_steps == final_steps,
            "only the original owner status/failure changes; no new identity or End"
        );
        assert!(
            nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
                == (2_100_000, 2_100_030)
                && free_admission_source(
                    &platform.api.db.pool,
                    receipt.tenant.as_uuid(),
                    receipt.run.as_uuid(),
                    receipt.cutoff
                )
                .await
                    == receipt.source,
            "old L/V/effective bytes and complete immutable source remain exact"
        );
        let final_events = final_state["events"].as_array().expect("terminal journal");
        let drain_events = after["events"].as_array().expect("drain journal");
        assert!(
            final_events.len() == old_events.len() + 4
                && final_events.get(..drain_events.len()) == Some(drain_events.as_slice()),
            "exact two-event finish suffix preserves entire old and first-drain journals"
        );
        let terminal_stamp = final_events[final_events.len() - 2]["occurred_at"].clone();
        assert!(
            terminal_stamp.is_string(),
            "native finish timestamp present"
        );
        let mut expected_finish = envelope(
            receipt.head + 3,
            json!("c10"),
            owner_before["step_id"].clone(),
            json!([]),
            json!([]),
            "collection.finished",
            json!({"outcome":"failed","code":"RUN_DATA_LIMIT"}),
        );
        expected_finish["occurred_at"] = terminal_stamp.clone();
        let mut expected_run_finish = envelope(
            receipt.head + 4,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            "run.finished",
            json!({"status":"FAILED"}),
        );
        expected_run_finish["occurred_at"] = terminal_stamp;
        assert!(
            final_events[final_events.len() - 2] == expected_finish
                && final_events[final_events.len() - 1] == expected_run_finish,
            "exact native failed collection/Run envelopes and atomic final timestamp"
        );
        let drain_outbox = after["outbox"].as_array().expect("drain outbox");
        let final_outbox = final_state["outbox"].as_array().expect("terminal outbox");
        let appended = final_outbox
            .iter()
            .filter(|o| !drain_outbox.contains(o))
            .collect::<Vec<_>>();
        assert!(
            drain_outbox.iter().all(|o| final_outbox.contains(o))
                && final_outbox.len() == drain_outbox.len() + 2
                && appended.len() == 2,
            "exactly two finish hints preserve every previous row"
        );
        let collection_hint = appended
            .iter()
            .find(|o| o["event_type"] == "collection.finished")
            .expect("native collection finish hint");
        let run_hint = appended
            .iter()
            .find(|o| o["event_type"] == "run.finished")
            .expect("native Run finish hint");
        assert!(
            collection_hint["id"] == final_state["scope"]["finish_outbox_id"]
                && collection_hint["routing_key"] == "c10"
                && run_hint["routing_key"].is_null(),
            "actual finish hint identities and routing"
        );
        let expected_payload = json!({"tenantId":receipt.tenant.to_string(),"namespaceId":receipt.namespace.to_string(),"runId":receipt.run.to_string(),"status":"FAILED","traceId":before["run"]["trace_id"]});
        let mut expected_collection = expected_payload.clone();
        expected_collection["scopeId"] = json!(receipt.scope.to_string());
        assert!(
            collection_hint["payload"] == expected_collection
                && run_hint["payload"] == expected_payload,
            "truthful exact FAILED payloads with original scope/namespace/trace"
        );
        for hint in &appended {
            assert!(
                hint["tenant_id"] == receipt.tenant.to_string()
                    && hint["aggregate_id"] == receipt.run.to_string(),
                "native outbox tenant and aggregate match original Run"
            );
        }
        let terminal_projection =
            projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await;
        let fresh = PgRunStore::new(
            platform.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        )
        .with_scoped_worker_protocol();
        assert!(
            fresh.verify_run(receipt.tenant, receipt.run).await.is_ok(),
            "fresh store verifies mixed old/current failed history without output hydration"
        );
        let terminal = scheduler
            .advance_collection(receipt.tenant, receipt.run, &target)
            .await;
        assert!(
            matches!(terminal, Ok(CollectionAdvance::AlreadyFinished)),
            "terminal advance is already finished"
        );
        assert!(
            nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await == final_state
                && projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await
                    == terminal_projection,
            "fresh verify and terminal advance have no durable projection effects"
        );
        drop(fresh);
    }
    drop(scheduler);
    keeper.leave().await;
    drop(keeper);
    let role_pool = platform.api.api_pool.clone();
    let owner_pool = platform.api.db.pool.clone();
    drop(platform);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), role_pool.close())
            .await
            .is_ok(),
        "current role pool closes"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), owner_pool.close())
            .await
            .is_ok(),
        "current owner pool closes"
    );
}

async fn assert_equal_limit_admission(
    platform: &crate::support::worker::Platform,
    receipt: &FreeAdmissionReceipt,
    before: &Value,
    closed_before: &Value,
) {
    use sagat_application::execution::instance_step_id;
    use sagat_domain::execution::InstancePath;
    let after = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
    let entry = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("c10").expect("owner"), 0)
            .expect("item0 path"),
        NodeId::parse("legacyWork").expect("body entry"),
    );
    let step_id = instance_step_id(
        receipt
            .version
            .to_string()
            .parse()
            .expect("version identity"),
        receipt.run,
        &entry,
    );
    let old_events = before["events"].as_array().expect("old journal");
    let events = after["events"].as_array().expect("admission journal");
    assert!(
        events.len() == old_events.len() + 2
            && events.get(..old_events.len()) == Some(old_events.as_slice()),
        "admission preserves every historical event and appends exactly two"
    );
    let stamp = events[old_events.len()]["occurred_at"].clone();
    assert!(stamp.is_string(), "native admission timestamp");
    let owner_id = before["steps"]
        .as_array()
        .expect("old steps")
        .iter()
        .find(|s| s["node"] == "c10" && s["instance_path_nodes"] == json!([]))
        .expect("original root owner")["step_id"]
        .clone();
    let item = &before["items"][0];
    assert!(
        item["iteration_index"] == 0 && item["item_digest"].is_string(),
        "actual captured item0"
    );
    let item_hex = item["item_digest"]
        .as_str()
        .expect("actual bytea digest")
        .strip_prefix("\\x")
        .expect("PostgreSQL bytea hex");
    let envelope = |seq,
                    node: Value,
                    id: Value,
                    nodes: Value,
                    indices: Value,
                    kind,
                    detail: Value| {
        json!({"tenant_id":receipt.tenant.to_string(),"run_id":receipt.run.to_string(),"seq":seq,
        "node":node,"instance_id":id,"instance_path_nodes":nodes,"instance_path_indices":indices,
        "event_type":kind,"detail":detail,"actor":"system:engine","trace_id":before["run"]["trace_id"],
        "attempt":null,"payload_digest":null,"occurred_at":stamp})
    };
    assert!(
        events[old_events.len()]
            == envelope(
                receipt.head + 1,
                json!("c10"),
                owner_id,
                json!([]),
                json!([]),
                "collection.iteration_admitted",
                json!({"index":0,"entry":{"node":"legacyWork","path":[{"scope":"c10","index":0}]},"item_digest":item_hex})
            )
            && events[old_events.len() + 1]
                == envelope(
                    receipt.head + 2,
                    json!("legacyWork"),
                    json!(step_id.to_string()),
                    json!(["c10"]),
                    json!([0]),
                    "step.released",
                    json!({})
                ),
        "exact atomic admission/release envelopes and original trace"
    );
    let old_steps = before["steps"].as_array().expect("old steps");
    let steps = after["steps"].as_array().expect("new steps");
    let step = nonfinal_step(&after, "legacyWork");
    assert!(
        steps.len() == old_steps.len() + 1
            && steps.iter().filter(|s| *s != step).eq(old_steps.iter()),
        "one durable new instance and no changed original step or owner"
    );
    for (key, value) in step.as_object().expect("new step fields") {
        let expected = match key.as_str() {
            "tenant_id" => json!(receipt.tenant.to_string()),
            "run_id" => json!(receipt.run.to_string()),
            "namespace_id" => json!(receipt.namespace.to_string()),
            "step_id" => json!(step_id.to_string()),
            "node" => json!("legacyWork"),
            "status" => json!("READY"),
            "activity" => json!("line.reserve"),
            "instance_path_nodes" => json!(["c10"]),
            "instance_path_indices" => json!([0]),
            "kind" => json!("FORWARD"),
            "node_kind" => json!("legacyControl"),
            "ready_at" | "last_hinted_at" | "updated_at" => stamp.clone(),
            "attempt" | "generation" => json!(0),
            "hint_count" => json!(1),
            "cancel_requested" | "routed" | "timed_out" | "child_started" => json!(false),
            _ => Value::Null,
        };
        assert!(
            *value == expected,
            "new unclaimed READY instance exact field: {key}"
        );
    }
    let old_outbox = before["outbox"].as_array().expect("old outbox");
    let outbox = after["outbox"].as_array().expect("new outbox");
    let added = outbox
        .iter()
        .filter(|o| !old_outbox.contains(o))
        .collect::<Vec<_>>();
    assert!(
        outbox.len() == old_outbox.len() + 1
            && old_outbox.iter().all(|o| outbox.contains(o))
            && added.len() == 1,
        "exactly one task.ready outbox and unchanged complete old rows"
    );
    let hint = added[0];
    assert!(
        hint["id"]
            .as_str()
            .is_some_and(|s| Uuid::parse_str(s).is_ok())
            && hint["created_at"].is_string()
            && hint["next_attempt_at"].is_string(),
        "native outbox UUID and scheduling timestamps"
    );
    let expected_hint = json!({"id":hint["id"],"tenant_id":receipt.tenant.to_string(),"aggregate_id":receipt.run.to_string(),
        "event_type":"task.ready","routing_key":"line.reserve","payload":{"tenantId":receipt.tenant.to_string(),
        "namespaceId":receipt.namespace.to_string(),"runId":receipt.run.to_string(),"node":"legacyWork",
        "instancePath":[{"scope":"c10","index":0}],"activity":"line.reserve","traceId":before["run"]["trace_id"]},
        "created_at":hint["created_at"],"next_attempt_at":hint["next_attempt_at"],"attempt_count":0,"delivered_at":null,"last_error_code":null});
    assert!(
        *hint == expected_hint,
        "exact task.ready coordinates, payload and delivery state"
    );
    let member = json!({"tenant_id":receipt.tenant.to_string(),"run_id":receipt.run.to_string(),"scope_id":receipt.scope.to_string(),
        "iteration_index":0,"entry_path_nodes":["c10"],"entry_path_indices":[0],"entry_node":"legacyWork",
        "entry_instance_id":step_id.to_string(),"activity":"line.reserve","item_digest":item["item_digest"],
        "lifecycle":"ADMITTED","slot_held":true,"reserved_instances":1,"remaining_reservations":1,
        "return_path_nodes":null,"return_path_indices":null,"return_node":null,"return_instance_id":null,
        "result_digest":null,"admitted_sequence":receipt.head+1,"outbox_id":hint["id"],"checkpoint_sequence":null,
        "created_at":stamp,"updated_at":stamp,"drained_sequence":null,"drain_outbox_id":null});
    let mut expected = before.clone();
    expected["run"]["status"] = json!("RUNNING");
    expected["run"]["run_version"] = json!(receipt.head + 2);
    expected["run"]["updated_at"] = stamp.clone();
    for key in ["next_admission_index", "admitted_count", "active_count"] {
        expected["scope"][key] = json!(1);
    }
    expected["scope"]["updated_at"] = stamp;
    for key in ["admitted_instances", "charged_instances"] {
        expected["usage"][key] = json!(
            before["usage"][key]
                .as_i64()
                .expect("original instance counter")
                + 1
        );
    }
    expected["usage"]["reserved_instances"] = json!(1);
    expected["iteration_usage"]["active_iterations"] = json!(1);
    expected["members"] = json!([member]);
    expected["steps"] = after["steps"].clone();
    expected["events"] = after["events"].clone();
    expected["outbox"] = after["outbox"].clone();
    expected["telemetry"]["head_seq"] = json!(receipt.head + 2);
    let changed_fields = after
        .as_object()
        .expect("admission state fields")
        .iter()
        .filter(|(key, value)| expected[*key] != **value)
        .flat_map(|(key, value)| {
            if let Some(fields) = value.as_object() {
                fields
                    .iter()
                    .filter(|(field, v)| expected[key][*field] != **v)
                    .map(|(field, _)| format!("{key}.{field}"))
                    .collect::<Vec<_>>()
            } else {
                vec![key.clone()]
            }
        })
        .collect::<Vec<_>>();
    assert!(
        after == expected,
        "exact permitted admission delta; changed field names only: {changed_fields:?}"
    );
    let closed_after: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
        .bind(receipt.cutoff)
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("old closed source after admission");
    assert!(
        closed_after == *closed_before
            && nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
                == (2_097_137, 2_097_167),
        "closed old source and exact per-reference effective bytes remain unchanged"
    );
    let projection =
        projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await;
    let fresh = PgRunStore::new(
        platform.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    assert!(
        fresh.verify_run(receipt.tenant, receipt.run).await.is_ok(),
        "fresh store verifies genuine legacy equality admission"
    );
    assert!(
        nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await == after
            && projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await
                == projection,
        "cold verification has no durable effects on the admitted frontier"
    );
    println!(
        "current_equal_limit_admission L=2097137 V=2097167 E=4194304 admitted=0 ready=1 reserved=1 no_stop=true wire_key_observed=false"
    );
}

async fn assert_failed_member_flow(
    platform: &crate::support::worker::Platform,
    receipt: &FreeAdmissionReceipt,
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    target: &ExecutionInstanceId,
    before: &Value,
) {
    use sagat_scheduler::CollectionAdvance;
    let failed_entry = nonfinal_step(before, "legacyWork").clone();
    assert!(
        failed_entry["status"] == "FAILED"
            && failed_entry["routed"] == true
            && failed_entry["failure_code"] == "Q5_MEMBER_FAILURE"
            && failed_entry["output_digest"].is_null()
            && failed_entry["lease_token_hash"].is_null()
            && failed_entry["reported_lease_token_hash"].is_string()
            && before["scope"]["completed_count"] == 0
            && before["scope"]["next_admission_index"] == 1
            && before["members"][0]["slot_held"] == true
            && before["members"][0]["remaining_reservations"] == 1,
        "genuine terminal failed member still holds its original drain obligations"
    );
    let old_owner = before["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|s| s["node"] == "c10" && s["instance_path_nodes"] == json!([]))
        .expect("original collection owner")
        .clone();
    let mut previous = before.clone();
    for phase in 1..=3 {
        let result = scheduler
            .advance_collection(receipt.tenant, receipt.run, target)
            .await;
        assert!(
            matches!(
                (phase, result),
                (1, Ok(CollectionAdvance::DrainStarted))
                    | (2, Ok(CollectionAdvance::Drained { index: 0 }))
                    | (3, Ok(CollectionAdvance::DrainFinished))
            ),
            "exact normal failed-member drain transition"
        );
        let after = nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await;
        let old_events = previous["events"].as_array().expect("previous journal");
        let events = after["events"].as_array().expect("new journal");
        let added_count = if phase == 2 { 1 } else { 2 };
        assert!(
            events.len() == old_events.len() + added_count
                && events.get(..old_events.len()) == Some(old_events.as_slice()),
            "failed-member native suffix preserves complete historical journal"
        );
        let stamp = events[old_events.len()]["occurred_at"].clone();
        assert!(stamp.is_string(), "native transition timestamp");
        let envelope = |seq,
                        node: Value,
                        id: Value,
                        nodes: Value,
                        indices: Value,
                        kind,
                        detail: Value| {
            json!({
            "tenant_id":receipt.tenant.to_string(),"run_id":receipt.run.to_string(),"seq":seq,
            "node":node,"instance_id":id,"instance_path_nodes":nodes,"instance_path_indices":indices,
            "event_type":kind,"detail":detail,"actor":"system:engine","trace_id":before["run"]["trace_id"],
            "attempt":null,"payload_digest":null,"occurred_at":stamp})
        };
        let owner_event = |seq, kind, detail: Value| {
            envelope(
                seq,
                json!("c10"),
                old_owner["step_id"].clone(),
                json!([]),
                json!([]),
                kind,
                detail,
            )
        };
        let wanted = match phase {
            1 => vec![
                envelope(
                    receipt.head + 1,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    "run.stop_requested",
                    json!({"reason":"FAILED","code":"Q5_MEMBER_FAILURE"}),
                ),
                owner_event(
                    receipt.head + 2,
                    "collection.drain_started",
                    json!({"reason":"failed","iteration":0,"code":"Q5_MEMBER_FAILURE"}),
                ),
            ],
            2 => vec![owner_event(
                receipt.head + 3,
                "collection.iteration_drained",
                json!({"index":0}),
            )],
            _ => vec![
                owner_event(
                    receipt.head + 4,
                    "collection.finished",
                    json!({"outcome":"failed","code":"Q5_MEMBER_FAILURE"}),
                ),
                envelope(
                    receipt.head + 5,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    "run.finished",
                    json!({"status":"FAILED"}),
                ),
            ],
        };
        assert!(
            events[old_events.len()..] == wanted,
            "exact failed-member root/owner event envelopes and atomic timestamps"
        );
        let old_hints = previous["outbox"].as_array().expect("old outbox");
        let hints = after["outbox"].as_array().expect("new outbox");
        let new_hints = hints
            .iter()
            .filter(|h| !old_hints.contains(h))
            .collect::<Vec<_>>();
        let hint_count = if phase == 3 { 2 } else { 1 };
        assert!(
            hints.len() == old_hints.len() + hint_count
                && new_hints.len() == hint_count
                && old_hints.iter().all(|h| hints.contains(h)),
            "exact native hint count and unchanged complete old outbox rows"
        );
        for hint in &new_hints {
            let (kind, routing, scope_hint) = if phase == 1 {
                ("collection.drain.started", json!("c10"), true)
            } else if phase == 2 {
                ("collection.iteration.drained", json!("c10"), true)
            } else if hint["event_type"] == "collection.finished" {
                ("collection.finished", json!("c10"), true)
            } else {
                ("run.finished", Value::Null, false)
            };
            let mut payload = json!({"tenantId":receipt.tenant.to_string(),"namespaceId":receipt.namespace.to_string(),
                "runId":receipt.run.to_string(),"traceId":before["run"]["trace_id"]});
            if scope_hint {
                payload["scopeId"] = json!(receipt.scope.to_string());
            }
            if phase == 1 {
                payload["reason"] = json!("FAILED");
            } else if phase == 2 {
                payload["index"] = json!(0);
            } else {
                payload["status"] = json!("FAILED");
            }
            assert!(
                hint["id"]
                    .as_str()
                    .is_some_and(|v| Uuid::parse_str(v).is_ok())
                    && hint["created_at"].is_string()
                    && hint["next_attempt_at"].is_string(),
                "native hint UUID and timestamps"
            );
            let expected = json!({"id":hint["id"],"tenant_id":receipt.tenant.to_string(),"aggregate_id":receipt.run.to_string(),
                "event_type":kind,"routing_key":routing,"payload":payload,"created_at":hint["created_at"],
                "next_attempt_at":hint["next_attempt_at"],"attempt_count":0,"delivered_at":null,"last_error_code":null});
            assert!(
                **hint == expected,
                "exact truthful failed-member outbox envelope and delivery state"
            );
        }
        let mut expected = previous.clone();
        let head = receipt.head
            + match phase {
                1 => 2,
                2 => 3,
                _ => 5,
            };
        expected["run"]["run_version"] = json!(head);
        expected["run"]["updated_at"] = stamp.clone();
        expected["scope"]["updated_at"] = stamp.clone();
        expected["telemetry"]["head_seq"] = json!(head);
        expected["events"] = after["events"].clone();
        expected["outbox"] = after["outbox"].clone();
        if phase == 1 {
            expected["run"]["status"] = json!("CANCELLING");
            expected["run"]["stop_reason"] = json!("FAILED");
            expected["run"]["stop_code"] = json!("Q5_MEMBER_FAILURE");
            expected["scope"]["lifecycle"] = json!("DRAINING");
            expected["scope"]["drain_kind"] = json!("FAILED");
            expected["scope"]["drain_iteration"] = json!(0);
            expected["scope"]["drain_failure_code"] = json!("Q5_MEMBER_FAILURE");
            expected["scope"]["drain_sequence"] = json!(receipt.head + 2);
            expected["scope"]["drain_outbox_id"] = new_hints[0]["id"].clone();
        } else if phase == 2 {
            expected["scope"]["completed_count"] = json!(1);
            expected["scope"]["active_count"] = json!(0);
            expected["members"][0]["lifecycle"] = json!("DRAINED");
            expected["members"][0]["slot_held"] = json!(false);
            expected["members"][0]["remaining_reservations"] = json!(0);
            expected["members"][0]["drained_sequence"] = json!(receipt.head + 3);
            expected["members"][0]["drain_outbox_id"] = new_hints[0]["id"].clone();
            expected["members"][0]["updated_at"] = stamp.clone();
            expected["usage"]["reserved_instances"] = json!(0);
            expected["usage"]["retired_reservations"] = json!(
                previous["usage"]["retired_reservations"]
                    .as_i64()
                    .expect("retired reservation count")
                    + 1
            );
            expected["iteration_usage"]["active_iterations"] = json!(0);
        } else {
            expected["run"]["status"] = json!("FAILED");
            expected["run"]["finished_at"] = stamp.clone();
            expected["scope"]["lifecycle"] = json!("FAILED");
            expected["scope"]["finish_sequence"] = json!(receipt.head + 4);
            let finished = new_hints
                .iter()
                .find(|h| h["event_type"] == "collection.finished")
                .expect("actual scope finish hint");
            assert!(
                new_hints.iter().any(|h| h["event_type"] == "run.finished"),
                "actual final Run hint"
            );
            expected["scope"]["finish_outbox_id"] = finished["id"].clone();
            let owner = expected["steps"]
                .as_array_mut()
                .expect("steps")
                .iter_mut()
                .find(|s| s["step_id"] == old_owner["step_id"])
                .expect("same original owner");
            owner["status"] = json!("FAILED");
            owner["waiting_kind"] = Value::Null;
            owner["failure_code"] = json!("Q5_MEMBER_FAILURE");
            owner["updated_at"] = stamp;
        }
        let changed = after
            .as_object()
            .expect("state fields")
            .iter()
            .filter(|(k, v)| expected[*k] != **v)
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        assert!(
            after == expected,
            "exact permitted failed-member delta; changed field names only: {changed:?}"
        );
        assert!(
            nonfinal_step(&after, "legacyWork") == &failed_entry
                && nonfinal_bytes(&platform.api.db.pool, receipt.tenant, receipt.run).await
                    == (2_100_000, 2_100_030)
                && admission_source(
                    &platform.api.db.pool,
                    receipt.tenant.as_uuid(),
                    receipt.run.as_uuid(),
                    receipt.cutoff,
                    true
                )
                .await
                    == receipt.source,
            "original failure/report/closed source and effective bytes remain exact through drain"
        );
        let limit_audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 AND action='run.data_limit_exceeded'")
            .bind(receipt.tenant.as_uuid()).bind(receipt.run.to_string()).fetch_one(&platform.api.db.pool).await.expect("actual limit audit count");
        assert!(
            limit_audits == 0,
            "failed member wins without any data-limit audit"
        );
        previous = after;
    }
    let projection =
        projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await;
    let fresh = PgRunStore::new(
        platform.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    assert!(
        fresh.verify_run(receipt.tenant, receipt.run).await.is_ok(),
        "fresh store verifies old real failure and native terminal drain"
    );
    assert!(
        matches!(
            scheduler
                .advance_collection(receipt.tenant, receipt.run, target)
                .await,
            Ok(CollectionAdvance::AlreadyFinished)
        ),
        "terminal failed-member advance is already finished"
    );
    assert!(
        nonfinal_state(&platform.api.db.pool, receipt.tenant, receipt.run).await == previous
            && projection_witness(&platform.api.db.pool, receipt.tenant, &[receipt.run]).await
                == projection,
        "fresh verification and terminal replay preserve all durable projections"
    );
    println!(
        "current_failed_member_precedence L=2100000 V=2100030 E=4200030 original_failure=Q5_MEMBER_FAILURE terminal=FAILED data_limit_audits=0"
    );
}

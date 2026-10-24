//! Test-only frozen producer overlaid onto immutable `6c44a0e4` by the phased runner.
//!
//! Every durable execution fact is created through that commit's public REST/gRPC API and normal
//! scheduler. SQL is read-only and supplies the compatibility witnesses handed to the current
//! process. The final ordinary report is deliberately retained, not submitted here.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use prost::Message as _;
use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::TaskLease;
use sagat_application::entropy::{SystemEntropy, Token};
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::RunId;
use sagat_domain::identity::Role;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::worker::{NAMESPACE, Platform, TENANT, Worker, platform_with};

const FORMAT: &str = "q10c-ordinary-before-drain-v1";
const UNDER_LIMIT_FORMAT: &str = "q10c-ordinary-under-limit-increment-v1";
const OLD_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";
const WORKER_SUBJECT: &str = "q10c-ordinary-worker";
const ACTIVITIES: [&str; 3] = ["line.work", "line.reserve", "line.release"];
const SCHEDULER_OWNER: &str = "scope-compensation-q10c-ordinary-before-drain-v1";

#[derive(Clone, Copy)]
enum FixturePreset {
    Ordinary,
    UnderLimitIncrement,
}

impl FixturePreset {
    const fn format(self) -> &'static str {
        match self {
            Self::Ordinary => FORMAT,
            Self::UnderLimitIncrement => UNDER_LIMIT_FORMAT,
        }
    }

    const fn workflow_name(self) -> &'static str {
        match self {
            Self::Ordinary => "q10c-ordinary-before-drain-v1",
            Self::UnderLimitIncrement => "q10c-ordinary-under-limit-increment-v1",
        }
    }

    const fn worker_subject(self) -> &'static str {
        match self {
            Self::Ordinary => WORKER_SUBJECT,
            Self::UnderLimitIncrement => "q10c-ordinary-under-limit-increment-worker",
        }
    }

    const fn scheduler_owner(self) -> &'static str {
        match self {
            Self::Ordinary => SCHEDULER_OWNER,
            Self::UnderLimitIncrement => {
                "scope-compensation-q10c-ordinary-under-limit-increment-v1"
            }
        }
    }

    const fn report_key_prefix(self) -> &'static str {
        match self {
            Self::Ordinary => "q10c-ordinary",
            Self::UnderLimitIncrement => "q10c-ordinary-under-limit-increment",
        }
    }

    const fn source_string_bytes(self) -> usize {
        match self {
            Self::Ordinary => 209_998,
            Self::UnderLimitIncrement => 199_998,
        }
    }

    const fn source_output_bytes(self) -> usize {
        match self {
            Self::Ordinary => 210_000,
            Self::UnderLimitIncrement => 200_000,
        }
    }

    const fn source_aggregate_bytes(self) -> usize {
        match self {
            Self::Ordinary => 210_002,
            Self::UnderLimitIncrement => 200_002,
        }
    }

    const fn expected_totals(self) -> (i64, i64, i64) {
        match self {
            Self::Ordinary => (2_100_000, 2_100_020, 4_200_020),
            Self::UnderLimitIncrement => (2_000_000, 2_000_020, 4_000_020),
        }
    }

    fn completion_json(self) -> String {
        match self {
            Self::Ordinary => "1".to_owned(),
            Self::UnderLimitIncrement => serde_json::to_string(&Value::String("y".repeat(249_998)))
                .expect("fixed under-limit completion JSON"),
        }
    }
}

const PROJECTION_WITNESS_SQL: &str = r#"WITH selected_runs AS (
  SELECT run.id FROM runs run WHERE run.tenant_id=$1 AND run.id=$2
) SELECT jsonb_build_object(
  'runs',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM runs row_data JOIN selected_runs selected ON selected.id=row_data.id
    WHERE row_data.tenant_id=$1),
  'usage',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id)
    FROM run_instance_usage row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'scopes',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_path_nodes,row_data.owner_path_indices,row_data.owner_node)
    FROM iteration_scopes row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'steps',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node)
    FROM steps row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'events',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.seq)
    FROM run_events row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'item_sources',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_scope_items row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'iterations',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_iterations row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'checkpoints',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM iteration_checkpoints row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'payloads',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex'))
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT run.input_digest FROM runs run JOIN selected_runs selected ON selected.id=run.id
        WHERE run.tenant_id=$1
      UNION SELECT step.output_digest FROM steps step JOIN selected_runs selected ON selected.id=step.run_id
        WHERE step.tenant_id=$1
      UNION SELECT scope.snapshot_digest FROM iteration_scopes scope
        JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT scope.result_digest FROM iteration_scopes scope
        JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT item.item_digest FROM collection_scope_items item
        JOIN selected_runs selected ON selected.id=item.run_id WHERE item.tenant_id=$1
      UNION SELECT member.item_digest FROM collection_iterations member
        JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT member.result_digest FROM collection_iterations member
        JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint
        JOIN selected_runs selected ON selected.id=checkpoint.run_id WHERE checkpoint.tenant_id=$1
      UNION SELECT event.payload_digest FROM run_events event
        JOIN selected_runs selected ON selected.id=event.run_id WHERE event.tenant_id=$1)),
  'workflow_versions',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM workflow_versions row_data WHERE row_data.tenant_id=$1 AND row_data.id IN (
      SELECT run.workflow_version_id FROM runs run JOIN selected_runs selected ON selected.id=run.id
        WHERE run.tenant_id=$1)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data
    JOIN selected_runs selected ON selected.id=row_data.run_id WHERE row_data.tenant_id=$1),
  'outbox',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM event_outbox row_data JOIN selected_runs selected ON selected.id=row_data.aggregate_id
    WHERE row_data.tenant_id=$1))"#;

const CLOSED_SOURCE_SQL: &str = r#"WITH selected_scopes AS (
  SELECT scope.* FROM iteration_scopes scope
   WHERE scope.tenant_id=$1 AND scope.run_id=$2
     AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[]
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
        ON scope.tenant_id=item.tenant_id AND scope.run_id=item.run_id AND scope.scope_id=item.scope_id
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
    WHERE row_data.tenant_id=$1 AND row_data.run_id=$2 AND row_data.owner_node=ANY($3::text[]))
)"#;

fn document(preset: FixturePreset) -> Value {
    let roots = (0..11)
        .map(|index| {
            let next = if index == 10 {
                "done".to_owned()
            } else {
                format!("c{}", index + 1)
            };
            json!({
                "id": format!("c{index}"), "type": "forEach", "items": "input.lines",
                "body": if index == 10 { "effect" } else { "legacy" },
                "result": "bodyResult", "maxItems": 1, "maxConcurrency": 1, "next": next
            })
        })
        .chain(std::iter::once(json!({"id": "done", "type": "end"})))
        .collect::<Vec<_>>();
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": preset.workflow_name()}, "entrypoint": "c0",
        "bodies": {
            "legacy": {"entrypoint": "work", "nodes": [
                {"id": "work", "type": "activity", "activity": "line.work", "next": "returned"},
                {"id": "returned", "type": "return", "result": "steps.work.output"}
            ]},
            "effect": {"entrypoint": "reserve", "nodes": [
                {"id": "reserve", "type": "activity", "activity": "line.reserve",
                 "compensateWith": "line.release", "next": "returned"},
                {"id": "returned", "type": "return", "result": "1"}
            ]}
        },
        "nodes": roots
    })
}

fn digest(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("canonical witness JSON"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn projection_witness(platform: &Platform, run: RunId) -> String {
    let value: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(platform.tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("bounded old projection witness");
    digest(&value)
}

fn source_nodes() -> Vec<String> {
    (0..10).map(|index| format!("c{index}")).collect()
}

async fn source_witness(platform: &Platform, run: RunId) -> String {
    let cutoff: i64 = sqlx::query_scalar(
        "SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type='step.routed' AND node='c9' \
           AND cardinality(instance_path_nodes)=0",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("closed source cutoff");
    let value: Value = sqlx::query_scalar(CLOSED_SOURCE_SQL)
        .bind(platform.tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(source_nodes())
        .bind(cutoff)
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("bounded closed source witness");
    digest(&value)
}

async fn drive(
    scheduler: &Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>,
) {
    for _ in 0..80 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "old scheduler action failed: {report:?}");
        if !report.did_work() {
            return;
        }
    }
    panic!("old scheduler did not quiesce within the bounded drive");
}

fn assert_lease(lease: &TaskLease, owner: &str, node: &str, activity: &str) {
    assert_eq!(lease.task_kind, "FORWARD");
    assert_eq!(lease.activity_name, activity);
    let instance = lease.instance.as_ref().expect("scoped instance identity");
    assert_eq!(instance.definition_node_id, node);
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, owner);
    assert_eq!(instance.instance_path[0].iteration_index, 0);
}

async fn assert_old_shape(
    platform: &Platform,
    run: RunId,
    final_lease: &TaskLease,
    worker_id: &str,
    session: &str,
    preset: FixturePreset,
) {
    let totals: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT usage.logical_output_bytes, \
           COALESCE(sum(octet_length(payload.content)) \
             FILTER (WHERE owner.output_digest IS NULL),0)::bigint, \
           count(*) FILTER (WHERE scope.lifecycle='SUCCEEDED'), \
           count(*) FILTER (WHERE owner.status='SUCCEEDED' AND owner.output_digest IS NULL), \
           (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
             AND event_type='collection.finished' AND node=ANY($3::text[])), \
           (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
             AND event_type='step.routed' AND node=ANY($3::text[]) \
             AND cardinality(instance_path_nodes)=0) \
         FROM run_instance_usage usage \
         JOIN iteration_scopes scope ON scope.tenant_id=usage.tenant_id AND scope.run_id=usage.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
           AND owner.instance_path_nodes='{}'::text[] \
           AND owner.instance_path_indices='{}'::integer[] AND owner.node=scope.owner_node \
         LEFT JOIN payloads payload ON payload.tenant_id=scope.tenant_id \
           AND payload.digest=scope.result_digest \
         WHERE usage.tenant_id=$1 AND usage.run_id=$2 \
           AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
           AND scope.owner_node=ANY($3::text[]) \
         GROUP BY usage.logical_output_bytes",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(source_nodes())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("old source totals");
    let expected = preset.expected_totals();
    assert_eq!(totals, (expected.0, expected.1, 10, 10, 10, 10));
    assert_eq!(totals.0 + totals.1, expected.2);

    let live: (String, i32, i64, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT step.status,step.attempt,step.generation, \
                step.step_id=$4 AND step.lease_token_hash=$5 AND step.leased_by=$6, \
                step.lease_expires_at>clock_timestamp(), \
                session.token_hash=$7 AND session.expires_at>clock_timestamp(), \
                scope.lifecycle='OPEN' \
           FROM steps step JOIN iteration_scopes scope \
             ON scope.tenant_id=step.tenant_id AND scope.run_id=step.run_id \
            AND scope.owner_node='c10' AND scope.owner_path_nodes='{}'::text[] \
            AND scope.owner_path_indices='{}'::integer[] \
           JOIN worker_sessions session ON session.tenant_id=step.tenant_id \
            AND session.id::text=step.leased_by \
          WHERE step.tenant_id=$1 AND step.run_id=$2 AND step.node='reserve' \
            AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] AND step.namespace_id=$3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(Uuid::parse_str(&platform.namespace_id().await).expect("namespace id"))
    .bind(Uuid::parse_str(&final_lease.step_id).expect("step id"))
    .bind(Token::hash_of(&final_lease.lease_token))
    .bind(worker_id)
    .bind(Token::hash_of(session))
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("old live lease and session");
    assert_eq!(
        (live.0.as_str(), live.1, live.2),
        ("LEASED", final_lease.attempt, final_lease.generation)
    );
    assert!(
        live.3 && live.4 && live.5 && live.6,
        "old authority must still be live"
    );
    assert!(
        final_lease.expires_at_epoch_ms
            > time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000,
        "wire lease deadline must still be live"
    );
}

fn publish_receipt(path: &std::path::Path, receipt: &Value) {
    let parent = path.parent().expect("receipt parent");
    let temporary = parent.join(format!(".receipt.{}.tmp", Uuid::now_v7().simple()));
    let bytes = serde_json::to_vec(receipt).expect("serialize private receipt");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .expect("create private receipt atomically");
    file.write_all(&bytes).expect("write private receipt");
    file.sync_all().expect("sync private receipt");
    std::fs::rename(&temporary, path).expect("publish private receipt atomically");
    std::fs::File::open(parent)
        .expect("open private receipt directory")
        .sync_all()
        .expect("sync private receipt directory");
}

#[tokio::test]
async fn frozen_old_ordinary_success_before_drain_producer() {
    produce(FixturePreset::Ordinary, false).await;
}

#[tokio::test]
async fn frozen_old_ordinary_under_limit_increment_producer() {
    produce(FixturePreset::UnderLimitIncrement, false).await;
}

#[tokio::test]
async fn frozen_old_live_work_frontier_guard_red() {
    produce(FixturePreset::Ordinary, true).await;
}

async fn produce(preset: FixturePreset, live_frontier_red: bool) {
    assert_eq!(env!("SAGAT_Q10C_SOURCE_COMMIT"), OLD_COMMIT);
    assert!(std::env::var_os("SAGAT_TEST_DATABASE_URL").is_some());
    let receipt_path = std::env::var_os("SAGAT_Q10C_ORDINARY_PHASE_RECEIPT")
        .map(std::path::PathBuf::from)
        .expect("phased runner receipt path");
    assert!(!receipt_path.exists(), "private receipt must be create-new");

    let platform = platform_with(Options {
        runtime_role: crate::support::as_runtime_role(),
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(3_600),
            session_ttl: Duration::from_secs(3_600),
            ..WorkerSettings::default()
        },
        ..Options::default()
    })
    .await;
    let producer_api_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.api_pool)
        .await
        .expect("producer API database role");
    let namespace_id = platform.namespace_id().await;

    platform
        .grant(platform.tenant, preset.worker_subject(), Role::Worker)
        .await;
    let binding = platform
        .api
        .as_user(
            Method::PUT,
            &format!(
                "/namespaces/{namespace_id}/worker-bindings/{}",
                preset.worker_subject()
            ),
            "alice",
            TENANT,
        )
        .json(&json!({"activities": ACTIVITIES}))
        .send()
        .await
        .expect("create exact old worker binding");
    assert_eq!(binding.status(), StatusCode::OK);

    let quota = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 4})),
        )
        .await;
    assert_eq!(quota.0, 200, "{}", quota.1);
    let workflow = platform
        .publish(preset.workflow_name(), &document(preset))
        .await;
    let run: RunId = platform
        .start(&workflow, &json!({"lines": [1]}))
        .await
        .parse()
        .expect("run id");

    let placement = Placement::new(preset.scheduler_owner().to_owned());
    let round = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("acquire old scheduler authority");
    assert_eq!(round.replicas, 1);
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 64))
    .with_placement(placement);

    let mut worker = platform.connect(preset.worker_subject(), TENANT).await;
    worker.register_version(&ACTIVITIES, "1.5").await;
    let worker_id = worker.worker_id.clone();
    let session_token = worker.session.clone();
    let large = Value::String("x".repeat(preset.source_string_bytes()));
    assert_eq!(
        serde_json::to_vec(&large).expect("large JSON").len(),
        preset.source_output_bytes()
    );
    assert!(preset.source_output_bytes() < 262_144);
    assert_eq!(
        serde_json::to_vec(&vec![large.clone()])
            .expect("fixed single-member aggregate JSON")
            .len(),
        preset.source_aggregate_bytes()
    );
    assert!(preset.source_aggregate_bytes() < 262_144);

    for index in 0..10 {
        drive(&scheduler).await;
        let lease = worker.one_task().await;
        assert_lease(&lease, &format!("c{index}"), "work", "line.work");
        let ack = worker
            .complete(&lease, &large)
            .await
            .expect("complete old ordinary source member");
        assert!(!ack.already_applied);
        assert_eq!(ack.step_status, "SUCCEEDED");
    }
    drive(&scheduler).await;
    let final_lease = worker.one_task().await;
    assert_lease(&final_lease, "c10", "reserve", "line.reserve");
    let completion_json = preset.completion_json();
    assert!(completion_json.len() < 262_144);
    let expected_totals = preset.expected_totals();
    assert!(
        expected_totals.0 + i64::try_from(completion_json.len()).expect("candidate length")
            < 4 * 1024 * 1024
    );
    if matches!(preset, FixturePreset::UnderLimitIncrement) {
        assert_eq!(completion_json.len(), 250_000);
        assert_eq!(
            expected_totals.0 + i64::try_from(completion_json.len()).expect("candidate length"),
            2_250_000
        );
        assert_eq!(
            expected_totals.2 + i64::try_from(completion_json.len()).expect("candidate length"),
            4_250_020
        );
        assert!(
            expected_totals.2 + i64::try_from(completion_json.len()).expect("candidate length")
                > 4 * 1024 * 1024
        );
    }
    let mut completion = Worker::complete_message(&final_lease, &completion_json);
    completion.idempotency_key = format!("{}-{}", preset.report_key_prefix(), final_lease.task_id);
    assert_old_shape(
        &platform,
        run,
        &final_lease,
        &worker_id,
        &session_token,
        preset,
    )
    .await;

    if live_frontier_red {
        use sagat_application::scope_frontier::ScopeFrontierStore as _;
        use sagat_domain::execution::{CollectionPhase, ExecutionInstanceId};
        use sagat_domain::workflow::NodeId;
        use sagat_scheduler::CollectionAdvance;
        let inspection_role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("old inspection database role");
        assert!(
            producer_api_role
                == if crate::support::as_runtime_role() {
                    crate::support::RUNTIME_ROLE
                } else {
                    inspection_role.as_str()
                },
            "actual selected old API and shared execution-store role"
        );
        println!(
            "old_live_work_frontier_role api={} store={} database={}",
            producer_api_role, producer_api_role, platform.api.db.name
        );
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("live scope owner"));
        let frontier = match platform
            .api
            .executions
            .collection_frontier(platform.tenant, run, &owner)
            .await
        {
            Ok(Some(value)) => value,
            _ => panic!("actual immutable-old live frontier"),
        };
        assert!(
            frontier.phase == CollectionPhase::Open
                && frontier.run.stop_reason.is_none()
                && frontier.item_count == 1
                && frontier.next_admission == 1
                && frontier.completed == 0
                && frontier.max_concurrency == 1
                && frontier.active.len() == 1,
            "old only-live-work frontier has no stop or admission capacity"
        );
        let stored:(bool,i64,bool,Uuid)=sqlx::query_as("SELECT r.stop_code IS NULL,(SELECT count(*) FROM iteration_checkpoints k WHERE k.tenant_id=s.tenant_id AND k.run_id=s.run_id AND k.scope_id=s.scope_id),(SELECT w.status='LEASED' AND NOT w.cancel_requested FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.step_id=$3),s.scope_id FROM runs r JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND cardinality(s.owner_path_nodes)=0 WHERE r.tenant_id=$1 AND r.id=$2")
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).bind(Uuid::parse_str(&final_lease.step_id).expect("original lease step id"))
            .fetch_one(&platform.api.db.pool).await.expect("old uncancelled original lease and no stop/checkpoint");
        assert!(
            stored.0 && stored.1 == 0 && stored.2 && stored.3 == frontier.scope_id.as_uuid(),
            "old exact live scope, uncancelled lease, absent stop code and checkpoints"
        );
        let member = &frontier.active[0];
        assert!(
            member.entry.node().as_str() == "reserve"
                && member.entry.path().segments().len() == 1
                && member.entry.path().segments()[0].scope_node().as_str() == "c10"
                && member.entry.path().segments()[0].iteration_index() == 0
                && member.steps.len() == 1
                && member
                    .steps
                    .get(&member.entry)
                    .is_some_and(|step| step.status()
                        == sagat_domain::execution::StepStatus::Leased
                        && !step.cancel_requested()),
            "frontier contains exactly the original leased uncancelled reserve"
        );
        assert!(
            member.index == 0
                && member.first_failure().is_none()
                && member.pending_return(&frontier.run.definition).is_none(),
            "old live member has neither failure nor pending Return"
        );
        drop(frontier);
        let before = projection_witness(&platform, run).await;
        let source = source_witness(&platform, run).await;
        let result = scheduler
            .advance_collection(platform.tenant, run, &owner)
            .await
            .expect("normal immutable-old live advance");
        assert!(
            projection_witness(&platform, run).await == before
                && source_witness(&platform, run).await == source,
            "old Waiting must leave execution and immutable source unchanged"
        );
        let waiting = matches!(result, CollectionAdvance::Waiting);
        println!("\nlive_frontier_red_database={}", platform.api.db.name);
        println!("observed_legacy_live_work_waiting={waiting}");
        assert!(
            matches!(result, CollectionAdvance::DrainStarted),
            "old live-work guard RED: expected DrainStarted instead of Waiting"
        );
        return;
    }

    let receipt = json!({
        "format": preset.format(),
        "source_commit": OLD_COMMIT,
        "database_name": platform.api.db.name,
        "producer_api_role": producer_api_role,
        "tenant_id": platform.tenant.to_string(),
        "other_tenant_id": platform.other_tenant.to_string(),
        "namespace_id": namespace_id,
        "namespace_slug": NAMESPACE,
        "worker_subject": preset.worker_subject(),
        "tenant_alias": TENANT,
        "worker_id": worker_id,
        "session_token": session_token,
        "run_id": run.to_string(),
        "selected_owner": "c10",
        "scheduler_owner": preset.scheduler_owner(),
        "task_lease_b64": base64::engine::general_purpose::STANDARD.encode(final_lease.encode_to_vec()),
        "completion_request_b64": base64::engine::general_purpose::STANDARD.encode(completion.encode_to_vec()),
        "producer_projection_witness_sha256": projection_witness(&platform, run).await,
        "closed_source_witness_sha256": source_witness(&platform, run).await,
    });
    publish_receipt(&receipt_path, &receipt);
}

// Applied prerequisite is separate from ordinary presets and their oracles.
async fn applied_authority_witness(platform: &Platform, worker_id: &str) -> String {
    let value: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
 'tenants',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM tenants t WHERE t.id=ANY($1::uuid[])),
 'memberships',(SELECT jsonb_agg(to_jsonb(m) ORDER BY m.subject) FROM memberships m WHERE m.tenant_id=$2 AND m.subject=$3),
 'bindings',(SELECT jsonb_agg(to_jsonb(b) ORDER BY b.pattern) FROM worker_bindings b WHERE b.tenant_id=$2 AND b.namespace_id=$4 AND b.subject=$3),
 'bindingPools',(SELECT jsonb_agg(to_jsonb(p) ORDER BY p.pool) FROM worker_binding_pools p WHERE p.tenant_id=$2 AND p.namespace_id=$4 AND p.subject=$3),
 'session',(SELECT to_jsonb(w) FROM worker_sessions w WHERE w.tenant_id=$2 AND w.id=$5))"#,
    )
    .bind(vec![platform.tenant.as_uuid(), platform.other_tenant.as_uuid()])
    .bind(platform.tenant.as_uuid())
    .bind("q10c-applied-handoff-worker")
    .bind(Uuid::parse_str(&platform.namespace_id().await).expect("namespace"))
    .bind(Uuid::parse_str(worker_id).expect("worker id"))
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("durable worker authority witness");
    digest(&value)
}

const APPLIED_STATE_SQL: &str = r#"SELECT jsonb_build_object(
 'run', r.status='RUNNING' AND r.stop_reason IS NULL AND r.stop_code IS NULL AND r.result_digest IS NULL,
 'scope', s.lifecycle='OPEN' AND s.item_count=1 AND s.next_admission_index=1 AND s.admitted_count=1 AND s.completed_count=0 AND s.active_count=1 AND s.result_digest IS NULL AND s.drain_kind IS NULL AND s.drain_iteration IS NULL AND s.drain_sequence IS NULL,
 'member', m.lifecycle='ADMITTED' AND m.slot_held AND m.reserved_instances>=1 AND m.remaining_reservations>=1 AND m.result_digest IS NULL,
 'forward', f.step_id=$4 AND f.status='WAITING' AND f.waiting_kind='RECONCILIATION' AND f.failure_code='OUTCOME_UNKNOWN' AND f.attempt=$6 AND f.generation=$7 AND NOT f.cancel_requested AND f.output_digest IS NULL AND f.reported_lease_token_hash=$8,
 'lookup', q.step_id=$5 AND q.status='LEASED' AND q.attempt=$9 AND q.generation=$10 AND q.lease_token_hash=$11 AND q.leased_by=$12 AND q.lease_expires_at>clock_timestamp() AND NOT q.cancel_requested AND q.output_digest IS NULL AND q.reported_lease_token_hash IS NULL,
 'session', w.token_hash=$13 AND w.expires_at>clock_timestamp() AND w.namespace_id=$3 AND w.subject='q10c-applied-handoff-worker' AND w.accepted_protocol_minor=5,
 'only_forward_lookup', (SELECT count(*)=2 FROM steps x WHERE x.tenant_id=$1 AND x.run_id=$2 AND x.instance_path_nodes=ARRAY['c10']::text[] AND x.instance_path_indices=ARRAY[0]::integer[]),
 'no_done', NOT EXISTS(SELECT 1 FROM steps x WHERE x.tenant_id=$1 AND x.run_id=$2 AND x.node='done'),
 'request', (SELECT count(*)=1 AND bool_and(e.detail->>'cause'='OUTCOME_UNKNOWN' AND (e.detail->>'attempt')::int=$6 AND (e.detail->>'generation')::bigint=$7) FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.node='reserve' AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[] AND e.event_type='reconciliation.requested'),
 'no_outcome', NOT EXISTS(SELECT 1 FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[] AND e.event_type IN ('reconciliation.concluded','step.completed','step.failed','step.cancelled')),
 'no_checkpoint', NOT EXISTS(SELECT 1 FROM iteration_checkpoints k WHERE k.tenant_id=$1 AND k.run_id=$2 AND k.scope_id=s.scope_id),
 'no_saga', NOT EXISTS(SELECT 1 FROM scope_sagas z WHERE z.tenant_id=$1 AND z.run_id=$2),
 'no_certificate', NOT EXISTS(SELECT 1 FROM leaf_collection_success_certifications z WHERE z.tenant_id=$1 AND z.run_id=$2),
 'no_lookup_receipt', NOT EXISTS(SELECT 1 FROM command_dedupe z WHERE z.tenant_id=$1 AND z.scope LIKE concat('step:',$2::text,':%') AND z.idempotency_key=$14)
)
FROM runs r
JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND s.owner_path_nodes='{}'::text[] AND s.owner_path_indices='{}'::integer[]
JOIN collection_iterations m ON m.tenant_id=s.tenant_id AND m.run_id=s.run_id AND m.scope_id=s.scope_id AND m.iteration_index=0
JOIN steps f ON f.tenant_id=r.tenant_id AND f.run_id=r.id AND f.namespace_id=$3 AND f.node='reserve' AND f.instance_path_nodes=ARRAY['c10']::text[] AND f.instance_path_indices=ARRAY[0]::integer[]
JOIN steps q ON q.tenant_id=r.tenant_id AND q.run_id=r.id AND q.namespace_id=$3 AND q.node='reserve~r' AND q.instance_path_nodes=ARRAY['c10']::text[] AND q.instance_path_indices=ARRAY[0]::integer[]
JOIN worker_sessions w ON w.tenant_id=r.tenant_id AND w.id::text=q.leased_by
WHERE r.tenant_id=$1 AND r.id=$2
"#;

enum AppliedEnd {
    Handoff,
    QuotaRed,
}

#[tokio::test]
async fn frozen_old_applied_handoff_producer() {
    produce_applied_handoff(AppliedEnd::Handoff).await;
}

#[tokio::test]
async fn frozen_old_applied_output_quota_guard_red() {
    produce_applied_handoff(AppliedEnd::QuotaRed).await;
}

async fn produce_applied_handoff(end: AppliedEnd) {
    let preset = FixturePreset::Ordinary;
    let activities = ["line.work", "line.reserve", "line.release", "line.lookup"];
    assert_eq!(env!("SAGAT_Q10C_SOURCE_COMMIT"), OLD_COMMIT);
    assert!(std::env::var_os("SAGAT_TEST_DATABASE_URL").is_some());
    let receipt_path = std::env::var_os("SAGAT_Q10C_ORDINARY_PHASE_RECEIPT")
        .map(std::path::PathBuf::from)
        .expect("phased runner receipt path");
    assert!(!receipt_path.exists(), "private receipt must be create-new");

    let platform = platform_with(Options {
        runtime_role: crate::support::as_runtime_role(),
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(3_600),
            session_ttl: Duration::from_secs(3_600),
            ..WorkerSettings::default()
        },
        ..Options::default()
    })
    .await;
    let producer_api_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.api_pool)
        .await
        .expect("producer API database role");
    let namespace_id = platform.namespace_id().await;

    platform
        .grant(platform.tenant, "q10c-applied-handoff-worker", Role::Worker)
        .await;
    let binding = platform
        .api
        .as_user(
            Method::PUT,
            &format!(
                "/namespaces/{namespace_id}/worker-bindings/{}",
                "q10c-applied-handoff-worker"
            ),
            "alice",
            TENANT,
        )
        .json(&json!({"activities": activities}))
        .send()
        .await
        .expect("create exact old worker binding");
    assert_eq!(binding.status(), StatusCode::OK);

    let quota = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 4})),
        )
        .await;
    assert_eq!(quota.0, 200, "{}", quota.1);
    let mut applied_document = document(preset);
    assert!(applied_document["bodies"]["effect"]["nodes"][0]["activity"] == "line.reserve");
    applied_document["metadata"]["name"] = json!("q10c-applied-handoff-v1");
    applied_document["bodies"]["effect"]["nodes"][0]["reconcileWith"] = json!("line.lookup");
    let workflow = platform
        .publish("q10c-applied-handoff-v1", &applied_document)
        .await;
    let run: RunId = platform
        .start(&workflow, &json!({"lines": [1]}))
        .await
        .parse()
        .expect("run id");

    let placement = Placement::new("scope-compensation-q10c-applied-handoff-v1".to_owned());
    let round = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("acquire old scheduler authority");
    assert_eq!(round.replicas, 1);
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 64))
    .with_placement(placement);

    let mut worker = platform
        .connect("q10c-applied-handoff-worker", TENANT)
        .await;
    worker.register_version(&activities, "1.5").await;
    let worker_id = worker.worker_id.clone();
    let session_token = worker.session.clone();
    let large = Value::String("x".repeat(preset.source_string_bytes()));
    assert_eq!(
        serde_json::to_vec(&large).expect("large JSON").len(),
        preset.source_output_bytes()
    );
    assert!(preset.source_output_bytes() < 262_144);
    assert_eq!(
        serde_json::to_vec(&vec![large.clone()])
            .expect("fixed single-member aggregate JSON")
            .len(),
        preset.source_aggregate_bytes()
    );
    assert!(preset.source_aggregate_bytes() < 262_144);

    for index in 0..10 {
        drive(&scheduler).await;
        let lease = worker.one_task().await;
        assert_lease(&lease, &format!("c{index}"), "work", "line.work");
        let ack = worker
            .complete(&lease, &large)
            .await
            .expect("complete old ordinary source member");
        assert!(!ack.already_applied);
        assert_eq!(ack.step_status, "SUCCEEDED");
    }
    drive(&scheduler).await;
    let final_lease = worker.one_task().await;
    assert_lease(&final_lease, "c10", "reserve", "line.reserve");
    let completion_json = preset.completion_json();
    assert!(completion_json.len() < 262_144);
    let expected_totals = preset.expected_totals();
    assert!(
        expected_totals.0 + i64::try_from(completion_json.len()).expect("candidate length")
            < 4 * 1024 * 1024
    );
    if matches!(preset, FixturePreset::UnderLimitIncrement) {
        assert_eq!(completion_json.len(), 250_000);
        assert_eq!(
            expected_totals.0 + i64::try_from(completion_json.len()).expect("candidate length"),
            2_250_000
        );
        assert_eq!(
            expected_totals.2 + i64::try_from(completion_json.len()).expect("candidate length"),
            4_250_020
        );
        assert!(
            expected_totals.2 + i64::try_from(completion_json.len()).expect("candidate length")
                > 4 * 1024 * 1024
        );
    }
    assert_old_shape(
        &platform,
        run,
        &final_lease,
        &worker_id,
        &session_token,
        preset,
    )
    .await;

    let forward = final_lease;
    let ack = worker
        .fail_with(&forward, "OUTCOME_UNKNOWN", false)
        .await
        .expect("real old ambiguous effect");
    assert!(!ack.already_applied);
    assert_eq!(ack.step_status, "WAITING");
    drive(&scheduler).await;
    let lookup = worker.one_task().await;
    assert_eq!(lookup.task_kind, "RECONCILIATION");
    assert_eq!(lookup.activity_name, "line.lookup");
    let instance = lookup.instance.as_ref().expect("actual scoped lookup");
    assert_eq!(instance.definition_node_id, "reserve~r");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    assert!(
        lookup.target_instance == forward.instance
            && lookup.target_node == "reserve"
            && lookup.target_idempotency_key == forward.idempotency_key
    );
    assert!(
        !lookup.idempotency_key.is_empty() && lookup.idempotency_key != forward.idempotency_key
    );
    let mut completion =
        Worker::complete_message(&lookup, r#"{"effect":"applied","output":{"hold":7}}"#);
    completion.idempotency_key = format!("q10c-applied-handoff-{}", lookup.task_id);
    let state: Value = sqlx::query_scalar(APPLIED_STATE_SQL)
        .bind(platform.tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(Uuid::parse_str(&namespace_id).expect("namespace"))
        .bind(Uuid::parse_str(&forward.step_id).expect("forward step"))
        .bind(Uuid::parse_str(&lookup.step_id).expect("lookup step"))
        .bind(forward.attempt)
        .bind(forward.generation)
        .bind(Token::hash_of(&forward.lease_token))
        .bind(lookup.attempt)
        .bind(lookup.generation)
        .bind(Token::hash_of(&lookup.lease_token))
        .bind(&worker_id)
        .bind(Token::hash_of(&session_token))
        .bind(&completion.idempotency_key)
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("real old Applied-ready state");
    for (name, valid) in state.as_object().expect("boolean state proofs") {
        assert!(
            valid.as_bool() == Some(true),
            "old Applied-ready invariant: {name}"
        );
    }
    if matches!(end, AppliedEnd::QuotaRed) {
        let authority_before = applied_authority_witness(&platform, &worker_id).await;
        let source_before = source_witness(&platform, run).await;
        let retained_request = completion.encode_to_vec();
        assert_eq!(completion.output_json.len(), 40);
        let lookups_before = platform.api.authentication_lookups();
        let request = worker.req(completion.clone());
        let ack = worker
            .client
            .complete_step(request)
            .await
            .expect("real immutable-old Applied report")
            .into_inner();
        assert!(
            !ack.already_applied && ack.step_status == "SUCCEEDED" && ack.run_status == "RUNNING",
            "actual old successful Applied ACK"
        );
        assert!(
            platform.api.authentication_lookups() > lookups_before,
            "old public report authenticates the original subject/tenant"
        );
        assert!(
            completion.encode_to_vec() == retained_request,
            "retained original request bytes"
        );
        assert!(
            applied_authority_witness(&platform, &worker_id).await == authority_before,
            "old completion does not rebind, grant or renew the original session"
        );
        assert!(
            source_witness(&platform, run).await == source_before,
            "closed legacy source unchanged by old Applied success"
        );

        let proof: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'run',r.status='RUNNING' AND r.stop_reason IS NULL AND r.stop_code IS NULL AND r.finished_at IS NULL AND r.result_digest IS NULL,
          'forward',f.status='SUCCEEDED' AND f.attempt=$5 AND f.generation=$6 AND f.reported_lease_token_hash=$7 AND NOT f.cancel_requested AND p.content=convert_to('{"hold":7}','UTF8') AND octet_length(p.content)=10,
          'lookup',q.status='SUCCEEDED' AND q.attempt=$8 AND q.generation=$9 AND q.reported_lease_token_hash=$10 AND q.output_digest IS NULL AND NOT q.cancel_requested,
          'scope',s.lifecycle='OPEN' AND s.result_digest IS NULL AND s.drain_kind IS NULL AND s.drain_sequence IS NULL,
          'return_pending',t.status='PENDING' AND t.output_digest IS NULL,
          'actual_applied',(SELECT count(*)=1 AND bool_and(e.detail->>'verdict'='applied' AND e.detail->'result'->>'kind'='succeeded') FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.event_type='reconciliation.concluded' AND e.node='reserve' AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[]),
          'lookup_completed',(SELECT count(*)=1 AND bool_and((e.detail->>'attempt')::int=$8 AND (e.detail->>'generation')::bigint=$9) FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.event_type='step.completed' AND e.node='reserve~r' AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[]),
          'no_forward_completion',NOT EXISTS(SELECT 1 FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.event_type='step.completed' AND e.node='reserve' AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[]),
          'no_refusal',NOT EXISTS(SELECT 1 FROM audit_records a WHERE a.tenant_id=$1 AND a.resource_type='run' AND a.resource_id=$2::text AND a.action='run.data_limit_exceeded'),
          'no_checkpoint',NOT EXISTS(SELECT 1 FROM iteration_checkpoints k WHERE k.tenant_id=$1 AND k.run_id=$2 AND k.scope_id=s.scope_id),
          'no_saga',NOT EXISTS(SELECT 1 FROM scope_sagas z WHERE z.tenant_id=$1 AND z.run_id=$2),
          'no_comp_or_done',NOT EXISTS(SELECT 1 FROM steps z WHERE z.tenant_id=$1 AND z.run_id=$2 AND z.node IN ('reserve~c','done')))
          FROM runs r
          JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND s.owner_path_nodes='{}'::text[] AND s.owner_path_indices='{}'::integer[]
          JOIN steps f ON f.tenant_id=r.tenant_id AND f.run_id=r.id AND f.step_id=$3
          JOIN steps q ON q.tenant_id=r.tenant_id AND q.run_id=r.id AND q.step_id=$4
          JOIN steps t ON t.tenant_id=r.tenant_id AND t.run_id=r.id AND t.node='returned' AND t.instance_path_nodes=ARRAY['c10']::text[] AND t.instance_path_indices=ARRAY[0]::integer[]
          JOIN payloads p ON p.tenant_id=f.tenant_id AND p.digest=f.output_digest
          WHERE r.tenant_id=$1 AND r.id=$2"#)
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid())
            .bind(Uuid::parse_str(&forward.step_id).expect("original forward step"))
            .bind(Uuid::parse_str(&lookup.step_id).expect("original lookup step"))
            .bind(forward.attempt).bind(forward.generation).bind(Token::hash_of(&forward.lease_token))
            .bind(lookup.attempt).bind(lookup.generation).bind(Token::hash_of(&lookup.lease_token))
            .fetch_one(&platform.api.db.pool).await.expect("actual old Applied bypass proof");
        for (name, valid) in proof.as_object().expect("old quota bypass proof flags") {
            assert!(
                valid.as_bool() == Some(true),
                "actual old Applied bypass invariant: {name}"
            );
        }
        let totals: (i64,i64,i64) = sqlx::query_as(r#"SELECT u.logical_output_bytes,
          sum(octet_length(p.content)) FILTER (WHERE o.output_digest IS NULL)::bigint,
          u.logical_output_bytes+sum(octet_length(p.content)) FILTER (WHERE o.output_digest IS NULL)::bigint
          FROM run_instance_usage u JOIN iteration_scopes s ON s.tenant_id=u.tenant_id AND s.run_id=u.run_id
          JOIN steps o ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node AND o.instance_path_nodes='{}'::text[] AND o.instance_path_indices='{}'::integer[]
          JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest
          WHERE u.tenant_id=$1 AND u.run_id=$2 AND s.lifecycle='SUCCEEDED' AND s.owner_path_nodes='{}'::text[] AND s.owner_path_indices='{}'::integer[] GROUP BY u.logical_output_bytes"#)
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("old real retained output totals");
        assert_eq!(totals, (2_100_010, 2_100_020, 4_200_030));
        let heads:(i64,i64,i64)=sqlx::query_as("SELECT r.run_version,(SELECT max(e.seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id),t.head_seq FROM runs r JOIN telemetry_pending t ON t.tenant_id=r.tenant_id AND t.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2")
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("actual old committed heads");
        assert!(
            heads.0 == heads.1
                && heads.0 == heads.2
                && ack.command_id == format!("{run}:{}", heads.0)
        );
        let prefix = Token::hash_of(&lookup.lease_token)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let scope = format!("step:{run}:scoped:3:c10:0;9:reserve~r:{prefix}");
        let mut fingerprint = Vec::new();
        let attempt = u32::try_from(lookup.attempt)
            .expect("original attempt")
            .to_be_bytes();
        let generation = u64::try_from(lookup.generation)
            .expect("original generation")
            .to_be_bytes();
        for part in [
            b"complete".as_slice(),
            attempt.as_slice(),
            generation.as_slice(),
            completion.output_json.as_bytes(),
        ] {
            fingerprint.extend_from_slice(&(part.len() as u64).to_be_bytes());
            fingerprint.extend_from_slice(part);
        }
        let receipt_rows:Vec<(Vec<u8>,String,time::OffsetDateTime)>=sqlx::query_as("SELECT request_hash,resource_id,created_at FROM command_dedupe WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3")
            .bind(platform.tenant.as_uuid()).bind(&scope).bind(&completion.idempotency_key).fetch_all(&platform.api.db.pool).await.expect("original actual Applied completion receipt");
        assert!(
            receipt_rows.len() == 1
                && receipt_rows[0].0 == Sha256::digest(&fingerprint).to_vec()
                && receipt_rows[0].1 == format!("SUCCEEDED|RUNNING|{}", heads.0)
        );
        let receipt_witness = digest(
            &json!({"tenant":platform.tenant.to_string(),"scope":scope,"key":completion.idempotency_key,"request_hash":receipt_rows[0].0,"resource":receipt_rows[0].1,"created_at":receipt_rows[0].2.to_string()}),
        );
        println!("applied_quota_receipt_sha256={receipt_witness}");
        let desired_refusal:bool=sqlx::query_scalar("SELECT coalesce(r.stop_reason='FAILED' AND r.stop_code='RUN_DATA_LIMIT',false) AND f.output_digest IS NULL FROM runs r JOIN steps f ON f.tenant_id=r.tenant_id AND f.run_id=r.id AND f.step_id=$3 WHERE r.tenant_id=$1 AND r.id=$2")
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).bind(Uuid::parse_str(&forward.step_id).expect("forward step")).fetch_one(&platform.api.db.pool).await.expect("required current known-effect refusal");
        let inspection_role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("old inspection role");
        assert!(
            producer_api_role
                == if crate::support::as_runtime_role() {
                    crate::support::RUNTIME_ROLE
                } else {
                    inspection_role.as_str()
                },
            "actual old API/shared-store role"
        );
        println!(
            "old_applied_quota_role api={} store={} database={}",
            producer_api_role, producer_api_role, platform.api.db.name
        );
        println!("applied_quota_red_database={}", platform.api.db.name);
        println!("observed_legacy_applied_quota_bypass=true");
        assert!(
            desired_refusal,
            "known Applied effect must survive while its over-budget output is refused with RUN_DATA_LIMIT"
        );
        return; // A RED-mode run must never publish a compatibility handoff, even if its guard passes.
    }
    let receipt = json!({
        "format": "q10c-applied-handoff-v1", "source_commit": OLD_COMMIT,
        "database_name": platform.api.db.name, "producer_api_role": producer_api_role,
        "tenant_id": platform.tenant.to_string(), "other_tenant_id": platform.other_tenant.to_string(),
        "namespace_id": namespace_id, "namespace_slug": NAMESPACE,
        "worker_subject": "q10c-applied-handoff-worker", "tenant_alias": TENANT,
        "worker_id": worker_id, "session_token": session_token, "run_id": run.to_string(),
        "selected_owner": "c10", "scheduler_owner": "scope-compensation-q10c-applied-handoff-v1",
        "task_lease_b64": base64::engine::general_purpose::STANDARD.encode(lookup.encode_to_vec()),
        "forward_task_lease_b64": base64::engine::general_purpose::STANDARD.encode(forward.encode_to_vec()),
        "completion_request_b64": base64::engine::general_purpose::STANDARD.encode(completion.encode_to_vec()),
        "producer_projection_witness_sha256": projection_witness(&platform, run).await,
        "closed_source_witness_sha256": source_witness(&platform, run).await,
        "producer_authority_witness_sha256": applied_authority_witness(&platform, &worker_id).await,
    });
    publish_receipt(&receipt_path, &receipt);
}

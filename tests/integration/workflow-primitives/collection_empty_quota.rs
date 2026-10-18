//! A zero-item collection still produces the retained JSON aggregate `[]` (two bytes). The
//! aggregate must obey the run's remaining logical-output budget even though it has no members.
//! The scenarios use public draft/publish/start and real gRPC worker reports; SQL only observes
//! the committed result.

use sagat_application::execution::{MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES};
use sagat_application::worker::MAX_TASK_INPUT_BYTES;
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::support::blob_collections::{Sched, platform, scheduler, start_inline, status_and_code};
use crate::support::worker::{Platform, Worker};

pub(crate) const PREFIX_COUNT: usize = 16;
const PREFIX_ACTIVITY: &str = "src.fetch";
const BODY_ACTIVITY: &str = "src.line";

fn exact_json_string(bytes: usize) -> Value {
    assert!(bytes >= 2, "a JSON string needs its quotes");
    let value = Value::String("x".repeat(bytes - 2));
    assert_eq!(value.to_string().len(), bytes, "exact serialized size");
    value
}

fn exact_empty_collection_source(bytes: usize) -> Value {
    let empty = json!({"items": [], "padding": ""});
    let overhead = empty.to_string().len();
    assert!(bytes >= overhead, "source object must fit its public shape");
    let value = json!({"items": [], "padding": "x".repeat(bytes - overhead)});
    assert_eq!(value.to_string().len(), bytes, "exact source size");
    value
}

pub(crate) fn empty_collection_after_prefix(name: &str) -> Value {
    let mut nodes = Vec::with_capacity(PREFIX_COUNT + 2);
    for index in 0..PREFIX_COUNT {
        let id = format!("source{index:02}");
        let next = if index + 1 == PREFIX_COUNT {
            "items".to_owned()
        } else {
            format!("source{:02}", index + 1)
        };
        nodes.push(json!({
            "id": id,
            "type": "activity",
            "activity": PREFIX_ACTIVITY,
            "next": next
        }));
    }
    nodes.push(json!({
        "id": "items",
        "type": "forEach",
        "items": "steps.source15.output.items",
        "body": "line",
        "result": "bodyResult",
        "maxItems": 1,
        "maxConcurrency": 1,
        "next": "done"
    }));
    nodes.push(json!({"id": "done", "type": "end"}));

    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "source00",
        "limits": {"maxInstances": 64, "maxDepth": 1},
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": BODY_ACTIVITY, "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": nodes
    })
}

fn two_empty_collections_after_prefix(name: &str) -> Value {
    let mut document = empty_collection_after_prefix(name);
    let nodes = document["nodes"]
        .as_array_mut()
        .expect("workflow nodes are an array");
    nodes
        .iter_mut()
        .find(|node| node["id"] == "items")
        .expect("first collection")["next"] = json!("second");
    let end = nodes
        .iter()
        .position(|node| node["id"] == "done")
        .expect("end node");
    nodes.insert(
        end,
        json!({
            "id": "second",
            "type": "forEach",
            "items": "steps.source15.output.items",
            "body": "line",
            "result": "bodyResult",
            "maxItems": 1,
            "maxConcurrency": 1,
            "next": "done"
        }),
    );
    document
}

async fn next_root_prefix_task(
    worker: &mut Worker<'_>,
    sched: &Sched,
    expected_index: usize,
) -> sagat_api_grpc::pb::TaskLease {
    for _ in 0..8 {
        let mut tasks = worker.poll(64).await;
        assert!(tasks.len() <= 1, "the prefix is a sequential root chain");
        if let Some(task) = tasks.pop() {
            assert_eq!(task.activity_name, PREFIX_ACTIVITY);
            assert_eq!(task.lease_token.len(), 64, "real public lease token");
            assert!(
                task.input_json.len() <= MAX_TASK_INPUT_BYTES,
                "source{expected_index:02} context is {} bytes, over the 8 MiB task bound",
                task.input_json.len()
            );
            let instance = task.instance.as_ref().expect("protocol 1.5 identity");
            assert!(instance.instance_path.is_empty(), "a root prefix activity");
            assert_eq!(
                instance.definition_node_id,
                format!("source{expected_index:02}")
            );
            return task;
        }
        let report = sched.tick().await;
        assert_eq!(report.failed, 0, "advance prefix: {report:?}");
    }
    panic!("source{expected_index:02} was not leased within the bounded driver");
}

pub(crate) async fn complete_prefix_over_grpc(
    worker: &mut Worker<'_>,
    sched: &Sched,
    output_sizes: &[usize; PREFIX_COUNT],
) {
    for (index, bytes) in output_sizes.iter().copied().enumerate() {
        let task = next_root_prefix_task(worker, sched, index).await;
        let output = if index + 1 == PREFIX_COUNT {
            exact_empty_collection_source(bytes)
        } else {
            exact_json_string(bytes)
        };
        let completed = worker
            .complete(&task, &output)
            .await
            .expect("complete actual root worker report over gRPC");
        assert_eq!(completed.step_status, "SUCCEEDED");
    }
}

async fn drive_to_terminal(p: &Platform, sched: &Sched, run: Uuid) -> (String, Option<String>) {
    for _ in 0..32 {
        let report = sched.tick().await;
        assert_eq!(
            report.failed, 0,
            "bounded collection finalization: {report:?}"
        );
        let observed = status_and_code(p, run).await;
        if matches!(
            observed.0.as_str(),
            "SUCCEEDED" | "FAILED" | "CANCELLED" | "COMPENSATED" | "PARTIAL" | "TIMED_OUT"
        ) {
            return observed;
        }
    }
    panic!("run did not become terminal in 32 scheduler passes");
}

pub(crate) async fn assert_prefix_outputs(p: &Platform, run: Uuid, expected_bytes: usize) {
    let row = sqlx::query(
        "SELECT count(*) AS output_count,coalesce(sum(octet_length(payload.content)),0)::bigint AS output_bytes, \
                (SELECT logical_output_bytes FROM run_instance_usage usage \
                  WHERE usage.tenant_id=$1 AND usage.run_id=$2) AS ledger_bytes \
         FROM steps step JOIN payloads payload \
           ON payload.tenant_id=step.tenant_id AND payload.digest=step.output_digest \
         WHERE step.tenant_id=$1 AND step.run_id=$2 \
           AND step.instance_path_nodes='{}' AND step.instance_path_indices='{}' \
           AND step.node LIKE 'source%' AND step.status='SUCCEEDED'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("committed public prefix outputs");
    assert_eq!(row.get::<i64, _>("output_count"), PREFIX_COUNT as i64);
    assert_eq!(
        row.get::<i64, _>("output_bytes"),
        expected_bytes as i64,
        "every logical prefix report is retained even when payload content deduplicates"
    );
    assert_eq!(
        row.get::<i64, _>("ledger_bytes"),
        expected_bytes as i64,
        "the public reports, rather than seeded accounting, consume the ledger"
    );
}

#[tokio::test]
async fn empty_collection_aggregate_obeys_remaining_run_bytes() {
    let p = platform().await;
    let name = "empty-collection-at-run-byte-limit";
    let workflow = p.publish(name, &empty_collection_after_prefix(name)).await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let sched = scheduler(&p, None).await;
    let mut worker = p.worker("empty-collection-quota-worker").await;
    worker
        .register_version(&[PREFIX_ACTIVITY, BODY_ACTIVITY], "1.5")
        .await;

    let outputs = [MAX_PAYLOAD_BYTES; PREFIX_COUNT];
    complete_prefix_over_grpc(&mut worker, &sched, &outputs).await;
    const { assert!(PREFIX_COUNT * MAX_PAYLOAD_BYTES == MAX_RUN_OUTPUT_BYTES) };
    assert_prefix_outputs(&p, run, MAX_RUN_OUTPUT_BYTES).await;

    let terminal = drive_to_terminal(&p, &sched, run).await;
    assert_eq!(
        terminal,
        ("FAILED".to_owned(), Some("RUN_DATA_LIMIT".to_owned())),
        "RED on the old finalizer: it incorrectly succeeds and retains [] after the prefix spent all 4 MiB"
    );

    let row = sqlx::query(
        "SELECT scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                scope.result_digest IS NULL AS no_aggregate, \
                owner.output_digest IS NULL AS no_owner_output, \
                COALESCE((SELECT bool_and(step.output_digest IS NULL) FROM steps step \
                  WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id \
                    AND step.instance_path_nodes='{}' AND step.instance_path_indices='{}' \
                    AND step.node='done'),true) AS no_end_output, \
                run.result_digest IS NULL AS no_run_result,usage.logical_output_bytes, \
                (SELECT count(*) FROM collection_iterations member \
                  WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id) AS members, \
                (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=run.tenant_id AND checkpoint.run_id=run.id) AS checkpoints, \
                (SELECT count(*) FROM steps step WHERE step.tenant_id=run.tenant_id \
                  AND step.run_id=run.id AND cardinality(step.instance_path_nodes)>0) AS scoped_steps, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type LIKE 'collection.iteration_%') AS iteration_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='collection.drain_started') AS drain_events, \
                (SELECT max(event.detail->>'reason') FROM run_events event \
                  WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                    AND event.event_type='collection.drain_started') AS drain_reason, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='collection.finished') AS finish_events, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
                  AND message.aggregate_id=run.id AND message.event_type='collection.drain.started') AS drain_outboxes, \
                (SELECT max(message.payload->>'reason') FROM event_outbox message \
                  WHERE message.tenant_id=run.tenant_id AND message.aggregate_id=run.id \
                    AND message.event_type='collection.drain.started') AS public_drain_reason, \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                  AND audit.resource_type='run' AND audit.resource_id=run.id::text \
                  AND audit.action='run.data_limit_exceeded') AS data_limit_audits, \
                (SELECT max(audit.reason) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                  AND audit.resource_type='run' AND audit.resource_id=run.id::text \
                  AND audit.action='run.data_limit_exceeded') AS audit_reason \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' AND scope.owner_node='items' \
         JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
          AND owner.instance_path_nodes='{}' AND owner.instance_path_indices='{}' AND owner.node='items' \
         JOIN run_instance_usage usage ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("bounded zero-member refusal");

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
    assert!(row.get::<bool, _>("no_aggregate"));
    assert!(row.get::<bool, _>("no_owner_output"));
    assert!(row.get::<bool, _>("no_end_output"));
    assert!(row.get::<bool, _>("no_run_result"));
    assert_eq!(
        row.get::<i64, _>("logical_output_bytes"),
        MAX_RUN_OUTPUT_BYTES as i64
    );
    assert_eq!(
        (
            row.get::<i64, _>("members"),
            row.get::<i64, _>("checkpoints"),
            row.get::<i64, _>("scoped_steps"),
            row.get::<i64, _>("iteration_events")
        ),
        (0, 0, 0, 0),
        "a zero-item refusal invents no member, checkpoint, scoped step or iteration event"
    );
    assert_eq!(row.get::<i64, _>("drain_events"), 1);
    assert_eq!(
        row.get::<Option<String>, _>("drain_reason").as_deref(),
        Some("run_data_limit")
    );
    assert_eq!(row.get::<i64, _>("finish_events"), 1);
    assert_eq!(row.get::<i64, _>("drain_outboxes"), 1);
    assert_eq!(
        row.get::<Option<String>, _>("public_drain_reason")
            .as_deref(),
        Some("FAILED"),
        "the existing public hint enum stays closed"
    );
    assert_eq!(row.get::<i64, _>("data_limit_audits"), 1);
    assert_eq!(
        row.get::<Option<String>, _>("audit_reason").as_deref(),
        Some("aggregate")
    );
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("the typed drain and terminal failure replay");
}

#[tokio::test]
async fn empty_collection_aggregate_fits_with_exactly_two_bytes_remaining() {
    let p = platform().await;
    let name = "empty-collection-with-two-run-bytes";
    let workflow = p.publish(name, &empty_collection_after_prefix(name)).await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let sched = scheduler(&p, None).await;
    let mut worker = p.worker("empty-collection-control-worker").await;
    worker
        .register_version(&[PREFIX_ACTIVITY, BODY_ACTIVITY], "1.5")
        .await;

    let mut outputs = [MAX_PAYLOAD_BYTES; PREFIX_COUNT];
    outputs[PREFIX_COUNT - 1] -= 2;
    complete_prefix_over_grpc(&mut worker, &sched, &outputs).await;
    let prefix_bytes = MAX_RUN_OUTPUT_BYTES - 2;
    assert_prefix_outputs(&p, run, prefix_bytes).await;

    assert_eq!(
        drive_to_terminal(&p, &sched, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    let row = sqlx::query(
        "SELECT scope.lifecycle,scope.result_digest AS aggregate_digest, \
                owner.output_digest AS owner_digest,ended.output_digest AS end_digest, \
                run.result_digest AS run_digest,aggregate.content,usage.logical_output_bytes, \
                (SELECT count(*) FROM collection_iterations member \
                  WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id) AS members, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type LIKE 'collection.iteration_%') AS iteration_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='collection.drain_started') AS drains \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' AND scope.owner_node='items' \
         JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
          AND owner.instance_path_nodes='{}' AND owner.instance_path_indices='{}' AND owner.node='items' \
         JOIN steps ended ON ended.tenant_id=run.tenant_id AND ended.run_id=run.id \
          AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' AND ended.node='done' \
         JOIN payloads aggregate ON aggregate.tenant_id=scope.tenant_id AND aggregate.digest=scope.result_digest \
         JOIN run_instance_usage usage ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("successful two-byte aggregate boundary");

    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(
        Some(row.get::<Vec<u8>, _>("aggregate_digest")),
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        "the collection owner retains the successful aggregate"
    );
    assert_eq!(row.get::<Option<Vec<u8>>, _>("end_digest"), None);
    assert_eq!(row.get::<Option<Vec<u8>>, _>("run_digest"), None);
    assert_eq!(row.get::<Vec<u8>, _>("content"), b"[]".to_vec());
    assert_eq!(
        row.get::<i64, _>("logical_output_bytes"),
        MAX_RUN_OUTPUT_BYTES as i64,
        "the empty aggregate consumes exactly the final two logical bytes"
    );
    assert_eq!(
        (
            row.get::<i64, _>("members"),
            row.get::<i64, _>("iteration_events"),
            row.get::<i64, _>("drains")
        ),
        (0, 0, 0)
    );
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("the exact-fit success replays");
}

#[tokio::test]
async fn legal_end_output_refuses_when_successful_empty_aggregate_leaves_one_run_byte() {
    let p = platform().await;
    let name = "collection-end-crosses-run-byte-limit";
    let mut document = empty_collection_after_prefix(name);
    document["nodes"]
        .as_array_mut()
        .expect("workflow nodes")
        .iter_mut()
        .find(|node| node["id"] == "done")
        .expect("terminal End")["outputExpr"] = json!("17");
    assert_eq!(json!(17).to_string().len(), 2, "End result is two bytes");
    let workflow = p.publish(name, &document).await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let sched = scheduler(&p, None).await;
    let mut worker = p.worker("collection-end-run-quota-worker").await;
    worker
        .register_version(&[PREFIX_ACTIVITY, BODY_ACTIVITY], "1.5")
        .await;

    const { assert!(PREFIX_COUNT * MAX_PAYLOAD_BYTES == MAX_RUN_OUTPUT_BYTES) };
    let mut outputs = [MAX_PAYLOAD_BYTES; PREFIX_COUNT];
    outputs[PREFIX_COUNT - 1] -= 3;
    assert!(
        outputs
            .iter()
            .all(|bytes| *bytes > 0 && *bytes <= MAX_PAYLOAD_BYTES),
        "every real worker output is positive and within the 256 KiB value ceiling"
    );
    complete_prefix_over_grpc(&mut worker, &sched, &outputs).await;
    let prefix_bytes = MAX_RUN_OUTPUT_BYTES - 3;
    assert_eq!(outputs.iter().sum::<usize>(), prefix_bytes);
    assert_prefix_outputs(&p, run, prefix_bytes).await;
    let rejected_end_payloads_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id=$1 AND content=$2")
            .bind(p.tenant.as_uuid())
            .bind(b"17".as_slice())
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the candidate End payload is absent before collection finalization");
    assert_eq!(rejected_end_payloads_before, 0);
    let expected_run_audit_actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_records \
           WHERE tenant_id=$1 AND resource_type='run' AND resource_id=$2::text \
           ORDER BY created_at,id",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("Run audit actions after the real worker prefix");
    assert!(
        !expected_run_audit_actions
            .iter()
            .any(|action| action == "run.storage_quota_exceeded"),
        "the real worker prefix did not exhaust stored-byte quota"
    );

    assert_eq!(
        drive_to_terminal(&p, &sched, run).await,
        ("FAILED".to_owned(), Some("RUN_DATA_LIMIT".to_owned())),
        "the retained [] fits, but the legal two-byte End result crosses the Run ceiling"
    );

    let row = sqlx::query(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code, \
                run.result_digest IS NULL AS no_run_result,scope.lifecycle, \
                scope.result_digest AS aggregate_digest,owner.status AS owner_status, \
                owner.failure_code AS owner_failure,owner.routed AS owner_routed, \
                owner.output_digest AS owner_digest,aggregate.content AS aggregate, \
                ended.status AS end_status,ended.failure_code AS end_code, \
                ended.output_digest IS NULL AS no_end_output,usage.logical_output_bytes, \
                telemetry.head_seq,run.run_version, \
                (SELECT count(*) FROM steps step WHERE step.tenant_id=run.tenant_id \
                  AND step.run_id=run.id AND step.output_digest IS NOT NULL) AS output_count, \
                (SELECT coalesce(sum(octet_length(output.content)),0)::bigint \
                  FROM steps step JOIN payloads output \
                    ON output.tenant_id=step.tenant_id AND output.digest=step.output_digest \
                  WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id) AS step_output_bytes, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='collection.finished' \
                  AND event.node='items' AND event.detail->>'outcome'='succeeded') AS collection_finished, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='step.failed' \
                  AND event.node='done' AND event.detail->>'code'='RUN_DATA_LIMIT') AS end_failed, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='end.reached') AS end_reached, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='run.stop_requested' \
                  AND event.detail->>'code'='RUN_DATA_LIMIT') AS stop_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='run.finished' \
                  AND event.detail->>'status'='FAILED') AS run_finished, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
                  AND message.aggregate_id=run.id \
                  AND message.event_type='collection.finished') AS collection_outboxes, \
                COALESCE((SELECT bool_and(message.routing_key='items' \
                  AND message.payload->>'status'='SUCCEEDED') FROM event_outbox message \
                  WHERE message.tenant_id=run.tenant_id AND message.aggregate_id=run.id \
                    AND message.event_type='collection.finished'),false) \
                  AS collection_outbox_exact, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
                  AND message.aggregate_id=run.id \
                  AND message.event_type='run.finished') AS run_outboxes, \
                COALESCE((SELECT bool_and(message.routing_key IS NULL \
                  AND message.payload->>'status'='FAILED') FROM event_outbox message \
                  WHERE message.tenant_id=run.tenant_id AND message.aggregate_id=run.id \
                    AND message.event_type='run.finished'),false) AS run_outbox_exact \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
            AND scope.owner_node='items' \
           JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
            AND owner.instance_path_nodes='{}' AND owner.instance_path_indices='{}' \
            AND owner.node='items' \
           JOIN steps ended ON ended.tenant_id=run.tenant_id AND ended.run_id=run.id \
            AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' \
            AND ended.node='done' \
           JOIN payloads aggregate ON aggregate.tenant_id=scope.tenant_id \
            AND aggregate.digest=scope.result_digest \
           JOIN run_instance_usage usage ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
           JOIN telemetry_pending telemetry ON telemetry.tenant_id=run.tenant_id \
            AND telemetry.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("successful aggregate and refused End result snapshot");

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
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(row.get::<String, _>("owner_status"), "SUCCEEDED");
    assert_eq!(row.get::<Option<String>, _>("owner_failure"), None);
    assert!(row.get::<bool, _>("owner_routed"));
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        Some(row.get::<Vec<u8>, _>("aggregate_digest")),
        "the successful owner retains the fitting aggregate"
    );
    assert_eq!(row.get::<Vec<u8>, _>("aggregate"), b"[]".to_vec());
    assert_eq!(row.get::<String, _>("end_status"), "FAILED");
    assert_eq!(
        row.get::<Option<String>, _>("end_code").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_end_output"));

    let retained_bytes = prefix_bytes + b"[]".len();
    assert_eq!(retained_bytes, MAX_RUN_OUTPUT_BYTES - 1);
    assert_eq!(
        row.get::<i64, _>("logical_output_bytes"),
        retained_bytes as i64,
        "only the real prefix and fitting owner aggregate charge the Run"
    );
    assert_eq!(
        row.get::<i64, _>("step_output_bytes"),
        retained_bytes as i64,
        "the ledger equals the complete set of retained step-output references"
    );
    assert_eq!(
        row.get::<i64, _>("output_count"),
        (PREFIX_COUNT + 1) as i64,
        "sixteen worker outputs plus the owner aggregate are retained"
    );
    assert_eq!(
        (
            row.get::<i64, _>("collection_finished"),
            row.get::<i64, _>("end_failed"),
            row.get::<i64, _>("end_reached"),
            row.get::<i64, _>("stop_events"),
            row.get::<i64, _>("run_finished"),
            row.get::<i64, _>("collection_outboxes"),
            row.get::<i64, _>("run_outboxes"),
        ),
        (1, 1, 0, 1, 1, 1, 1),
        "one successful collection and one failed terminal decision are committed"
    );
    assert!(
        row.get::<bool, _>("collection_outbox_exact"),
        "the sole collection.finished hint has the successful items routing"
    );
    assert!(
        row.get::<bool, _>("run_outbox_exact"),
        "the sole run.finished hint has the failed Run payload and no routing key"
    );
    let final_run_audit_actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_records \
           WHERE tenant_id=$1 AND resource_type='run' AND resource_id=$2::text \
           ORDER BY created_at,id",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("Run audit actions after the End budget refusal");
    assert_eq!(
        final_run_audit_actions, expected_run_audit_actions,
        "End logical-output refusal requires no new Run audit action"
    );
    assert!(
        !final_run_audit_actions
            .iter()
            .any(|action| action == "run.storage_quota_exceeded"),
        "a logical-output refusal is not a stored-byte quota event"
    );
    let rejected_end_payloads_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id=$1 AND content=$2")
            .bind(p.tenant.as_uuid())
            .bind(b"17".as_slice())
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the refused End payload is not retained as an orphan");
    assert_eq!(rejected_end_payloads_after, 0);
    assert_eq!(
        row.get::<i64, _>("head_seq"),
        row.get::<i64, _>("run_version"),
        "the complete terminal snapshot is visible at one telemetry head"
    );
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("the successful aggregate and failed End replay consistently");
}

#[tokio::test]
async fn second_empty_collection_refuses_after_first_consumes_final_two_bytes() {
    let p = platform().await;
    let name = "second-empty-collection-at-run-byte-limit";
    let workflow = p
        .publish(name, &two_empty_collections_after_prefix(name))
        .await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let sched = scheduler(&p, None).await;
    let mut worker = p.worker("two-empty-collections-quota-worker").await;
    worker
        .register_version(&[PREFIX_ACTIVITY, BODY_ACTIVITY], "1.5")
        .await;

    let mut outputs = [MAX_PAYLOAD_BYTES; PREFIX_COUNT];
    outputs[PREFIX_COUNT - 1] -= 2;
    complete_prefix_over_grpc(&mut worker, &sched, &outputs).await;
    let prefix_bytes = MAX_RUN_OUTPUT_BYTES - 2;
    assert_prefix_outputs(&p, run, prefix_bytes).await;

    assert_eq!(
        drive_to_terminal(&p, &sched, run).await,
        ("FAILED".to_owned(), Some("RUN_DATA_LIMIT".to_owned())),
        "the first [] spends the final two bytes, so the second [] must be refused"
    );

    let run_row = sqlx::query(
        "SELECT run.status,run.stop_reason,run.stop_code,run.result_digest IS NULL AS no_result, \
                usage.logical_output_bytes,telemetry.head_seq,run.run_version, \
                (SELECT count(*) FROM steps step WHERE step.tenant_id=run.tenant_id \
                  AND step.run_id=run.id AND step.instance_path_nodes='{}' \
                  AND step.instance_path_indices='{}' AND step.node='done') AS end_steps, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type='end.reached') AS end_events, \
                (SELECT count(*) FROM collection_scope_items item WHERE item.tenant_id=run.tenant_id \
                  AND item.run_id=run.id) AS items, \
                (SELECT count(*) FROM collection_iterations member WHERE member.tenant_id=run.tenant_id \
                  AND member.run_id=run.id) AS members, \
                (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=run.tenant_id AND checkpoint.run_id=run.id) AS checkpoints, \
                (SELECT count(*) FROM steps step WHERE step.tenant_id=run.tenant_id \
                  AND step.run_id=run.id AND cardinality(step.instance_path_nodes)>0) AS scoped_steps, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.event_type LIKE 'collection.iteration_%') AS iteration_events \
           FROM runs run JOIN run_instance_usage usage \
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
           JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("terminal run and usage projections");
    assert_eq!(run_row.get::<String, _>("status"), "FAILED");
    assert_eq!(
        run_row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("FAILED")
    );
    assert_eq!(
        run_row.get::<Option<String>, _>("stop_code").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(run_row.get::<bool, _>("no_result"));
    assert_eq!(
        run_row.get::<i64, _>("logical_output_bytes"),
        MAX_RUN_OUTPUT_BYTES as i64,
        "only the first retained [] charges two bytes"
    );
    assert_eq!(
        run_row.get::<i64, _>("head_seq"),
        run_row.get::<i64, _>("run_version"),
        "the terminal write advances the telemetry head atomically"
    );
    assert_eq!(
        (
            run_row.get::<i64, _>("end_steps"),
            run_row.get::<i64, _>("end_events"),
            run_row.get::<i64, _>("items"),
            run_row.get::<i64, _>("members"),
            run_row.get::<i64, _>("checkpoints"),
            run_row.get::<i64, _>("scoped_steps"),
            run_row.get::<i64, _>("iteration_events"),
        ),
        (0, 0, 0, 0, 0, 0, 0),
        "two empty collections invent no item, member, checkpoint, scoped step, iteration event or End"
    );

    let first = sqlx::query(
        "SELECT scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                scope.result_digest AS scope_digest,owner.status AS owner_status, \
                owner.failure_code AS owner_failure,owner.output_digest AS owner_digest, \
                payload.content,scope.finish_sequence,scope.finish_outbox_id \
           FROM iteration_scopes scope JOIN steps owner \
             ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
            AND owner.instance_path_nodes=scope.owner_path_nodes \
            AND owner.instance_path_indices=scope.owner_path_indices AND owner.node=scope.owner_node \
           JOIN payloads payload ON payload.tenant_id=scope.tenant_id \
            AND payload.digest=scope.result_digest \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_path_nodes='{}' \
            AND scope.owner_path_indices='{}' AND scope.owner_node='items'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("first empty collection result");
    let first_scope_digest = first.get::<Vec<u8>, _>("scope_digest");
    assert_eq!(first.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(first.get::<Option<String>, _>("drain_kind"), None);
    assert_eq!(first.get::<Option<String>, _>("drain_failure_code"), None);
    assert_eq!(first.get::<String, _>("owner_status"), "SUCCEEDED");
    assert_eq!(first.get::<Option<String>, _>("owner_failure"), None);
    assert_eq!(
        first.get::<Option<Vec<u8>>, _>("owner_digest").as_deref(),
        Some(first_scope_digest.as_slice()),
        "the successful owner retains the aggregate"
    );
    assert_eq!(first.get::<Vec<u8>, _>("content"), b"[]".to_vec());
    assert!(first.get::<i64, _>("finish_sequence") > 0);
    assert_ne!(first.get::<Uuid, _>("finish_outbox_id"), Uuid::nil());

    let second = sqlx::query(
        "SELECT scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                scope.result_digest IS NULL AS no_scope_result,owner.status AS owner_status, \
                owner.failure_code AS owner_failure,owner.output_digest IS NULL AS no_owner_output, \
                scope.finish_sequence,scope.finish_outbox_id \
           FROM iteration_scopes scope JOIN steps owner \
             ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
            AND owner.instance_path_nodes=scope.owner_path_nodes \
            AND owner.instance_path_indices=scope.owner_path_indices AND owner.node=scope.owner_node \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_path_nodes='{}' \
            AND scope.owner_path_indices='{}' AND scope.owner_node='second'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("second empty collection refusal");
    assert_eq!(second.get::<String, _>("lifecycle"), "FAILED");
    assert_eq!(
        second.get::<Option<String>, _>("drain_kind").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        second
            .get::<Option<String>, _>("drain_failure_code")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(second.get::<bool, _>("no_scope_result"));
    assert_eq!(second.get::<String, _>("owner_status"), "FAILED");
    assert_eq!(
        second.get::<Option<String>, _>("owner_failure").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(second.get::<bool, _>("no_owner_output"));
    assert!(second.get::<i64, _>("finish_sequence") > 0);
    assert_ne!(second.get::<Uuid, _>("finish_outbox_id"), Uuid::nil());

    let journal = sqlx::query(
        "SELECT \
           count(*) FILTER (WHERE event_type='collection.entered' AND node='items' \
             AND detail->>'item_count'='0') AS first_entered, \
           count(*) FILTER (WHERE event_type='collection.entered' AND node='second' \
             AND detail->>'item_count'='0') AS second_entered, \
           count(*) FILTER (WHERE event_type='collection.finished' AND node='items' \
             AND detail->>'outcome'='succeeded' \
             AND decode(detail->>'result_digest','hex')=$3) AS first_finished, \
           count(*) FILTER (WHERE event_type='collection.finished' AND node='second' \
             AND detail->>'outcome'='failed' AND detail->>'code'='RUN_DATA_LIMIT') AS second_finished, \
           count(*) FILTER (WHERE event_type='run.stop_requested' \
             AND detail->>'reason'='FAILED' AND detail->>'code'='RUN_DATA_LIMIT') AS stops, \
           count(*) FILTER (WHERE event_type='collection.drain_started' AND node='second' \
             AND detail->>'reason'='run_data_limit') AS drains, \
           count(*) FILTER (WHERE event_type='run.finished' \
             AND detail->>'status'='FAILED') AS run_finished, \
           count(*) FILTER (WHERE event_type='collection.entered') AS all_entered, \
           count(*) FILTER (WHERE event_type='collection.finished') AS all_collection_finished, \
           count(*) FILTER (WHERE event_type='run.stop_requested') AS all_stops, \
           count(*) FILTER (WHERE event_type='collection.drain_started') AS all_drains, \
           count(*) FILTER (WHERE event_type='run.finished') AS all_run_finished \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .bind(&first_scope_digest)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("truthful collection journal");
    assert_eq!(
        (
            journal.get::<i64, _>("first_entered"),
            journal.get::<i64, _>("second_entered"),
            journal.get::<i64, _>("first_finished"),
            journal.get::<i64, _>("second_finished"),
            journal.get::<i64, _>("stops"),
            journal.get::<i64, _>("drains"),
            journal.get::<i64, _>("run_finished"),
            journal.get::<i64, _>("all_entered"),
            journal.get::<i64, _>("all_collection_finished"),
            journal.get::<i64, _>("all_stops"),
            journal.get::<i64, _>("all_drains"),
            journal.get::<i64, _>("all_run_finished"),
        ),
        (1, 1, 1, 1, 1, 1, 1, 2, 2, 1, 1, 1)
    );

    let hints = sqlx::query(
        "SELECT \
           count(*) FILTER (WHERE event_type='collection.finished' AND routing_key='items' \
             AND payload->>'status'='SUCCEEDED') AS first_finished, \
           count(*) FILTER (WHERE event_type='collection.finished' AND routing_key='second' \
             AND payload->>'status'='FAILED') AS second_finished, \
           count(*) FILTER (WHERE event_type='collection.drain.started' AND routing_key='second' \
             AND payload->>'reason'='FAILED') AS drains, \
           count(*) FILTER (WHERE event_type='run.finished' \
             AND payload->>'status'='FAILED') AS run_finished, \
           count(*) FILTER (WHERE event_type IN \
             ('collection.finished','collection.drain.started','run.finished')) AS relevant_total \
         FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("truthful collection outbox hints");
    assert_eq!(
        (
            hints.get::<i64, _>("first_finished"),
            hints.get::<i64, _>("second_finished"),
            hints.get::<i64, _>("drains"),
            hints.get::<i64, _>("run_finished"),
            hints.get::<i64, _>("relevant_total"),
        ),
        (1, 1, 1, 1, 4)
    );

    let audit = sqlx::query(
        "SELECT count(*) AS records,max(audit.actor) AS actor,max(audit.reason) AS reason, \
                bool_and(audit.trace_id IS NOT DISTINCT FROM run.trace_id) AS matching_trace \
           FROM audit_records audit JOIN runs run \
             ON run.tenant_id=audit.tenant_id AND run.id=$2 \
          WHERE audit.tenant_id=$1 AND audit.resource_type='run' \
            AND audit.resource_id=$2::text AND audit.action='run.data_limit_exceeded'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("private fixed-vocabulary refusal audit");
    assert_eq!(audit.get::<i64, _>("records"), 1);
    assert_eq!(
        audit.get::<Option<String>, _>("actor").as_deref(),
        Some("system:engine")
    );
    assert_eq!(
        audit.get::<Option<String>, _>("reason").as_deref(),
        Some("aggregate")
    );
    assert!(
        audit
            .get::<Option<bool>, _>("matching_trace")
            .unwrap_or(false)
    );

    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("the successful first aggregate and second data-limit drain replay");
}

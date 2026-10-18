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

const PREFIX_COUNT: usize = 16;
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

fn empty_collection_after_prefix(name: &str) -> Value {
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

async fn complete_prefix_over_grpc(
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

async fn assert_prefix_outputs(p: &Platform, run: Uuid, expected_bytes: usize) {
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
        Some("collection aggregate data limit exceeded")
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

//! Public collection completion must honor the already declared End output contract.

use crate::support::scope_flow::{Flow, collection_document};
use sagat_application::execution::MAX_PAYLOAD_BYTES;
use serde_json::{Value, json};
use sqlx::Row;

fn decoded(value: Option<Vec<u8>>) -> Option<Value> {
    value.map(|bytes| serde_json::from_slice(&bytes).expect("retained JSON output"))
}

mod replay {
    use super::Flow;
    use sagat_application::{
        execution::{DefinitionCache, EndOutputAnswer},
        primitives::{
            CollectionFinish, CollectionScopeId, CollectionStore, CollectionTransition,
            PrimitiveStoreError, SchedulerAuthority,
        },
    };
    use sagat_domain::{
        execution::ExecutionInstanceId,
        identity::{NamespaceId, Subject},
        workflow::{Digest, NodeId},
    };
    use sagat_infra_ir::JsonIrCodec;
    use sagat_infra_postgres::PgCollectionStore;
    use serde_json::Value;
    use sqlx::{Row, postgres::PgRow};
    use std::sync::Arc;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[derive(Debug, Clone, Copy)]
    enum RecordedEndBranch {
        EvaluatedExpressionFailure,
        PreEvaluationInstanceLimit,
    }

    impl RecordedEndBranch {
        fn matching_answer(self) -> Option<EndOutputAnswer> {
            match self {
                Self::EvaluatedExpressionFailure => Some(EndOutputAnswer::ExpressionError),
                Self::PreEvaluationInstanceLimit => None,
            }
        }

        fn contradictory_answer(self) -> EndOutputAnswer {
            match self {
                Self::EvaluatedExpressionFailure => EndOutputAnswer::Succeeded(b"17".to_vec()),
                Self::PreEvaluationInstanceLimit => EndOutputAnswer::ExpressionError,
            }
        }

        fn assert_recorded(self, row: &PgRow) {
            assert_eq!(row.get::<String, _>("scope_lifecycle"), "SUCCEEDED");
            assert_eq!(row.get::<String, _>("owner_status"), "SUCCEEDED");
            assert!(row.get::<bool, _>("owner_routed"));
            assert_eq!(
                row.get::<Option<Vec<u8>>, _>("owner_digest"),
                row.get::<Option<Vec<u8>>, _>("result_digest"),
                "the successful collection owner retains its aggregate"
            );
            assert_eq!(row.get::<String, _>("run_status"), "FAILED");
            assert!(row.get::<Option<Vec<u8>>, _>("run_result").is_none());
            assert!(row.get::<Option<Vec<u8>>, _>("end_output").is_none());
            assert_eq!(row.get::<i64, _>("end_reached"), 0);
            match self {
                Self::EvaluatedExpressionFailure => {
                    assert_eq!(
                        row.get::<Option<String>, _>("run_code").as_deref(),
                        Some("EXPRESSION_ERROR")
                    );
                    assert_eq!(row.get::<String, _>("end_status"), "FAILED");
                    assert_eq!(
                        row.get::<Option<String>, _>("end_code").as_deref(),
                        Some("EXPRESSION_ERROR")
                    );
                }
                Self::PreEvaluationInstanceLimit => {
                    assert_eq!(
                        row.get::<Option<String>, _>("run_code").as_deref(),
                        Some("INSTANCE_LIMIT")
                    );
                    assert_eq!(row.get::<String, _>("end_status"), "CANCELLED");
                }
            }
        }
    }

    fn stored_digest(bytes: Vec<u8>) -> Digest {
        Digest::new(bytes.try_into().expect("stored SHA-256 digest"))
    }

    async fn original_finish(flow: &Flow, branch: RecordedEndBranch) -> CollectionFinish {
        let row = sqlx::query(
            "SELECT run.namespace_id,run.trace_id,run.status AS run_status, \
                    run.stop_code AS run_code,run.result_digest AS run_result, \
                    scope.scope_id,scope.lifecycle AS scope_lifecycle,scope.result_digest, \
                    scope.finish_outbox_id,scope.finish_sequence,scope.item_count, \
                    event.occurred_at,owner.status AS owner_status,owner.routed AS owner_routed, \
                    owner.output_digest AS owner_digest,ended.status AS end_status, \
                    ended.failure_code AS end_code,ended.output_digest AS end_output, \
                    shard.owner AS scheduler_owner,shard.generation AS scheduler_generation, \
                    (SELECT count(*) FROM run_events reached \
                      WHERE reached.tenant_id=run.tenant_id AND reached.run_id=run.id \
                        AND reached.event_type='end.reached') AS end_reached \
               FROM runs run JOIN iteration_scopes scope \
                 ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
               JOIN run_events event ON event.tenant_id=scope.tenant_id \
                AND event.run_id=scope.run_id AND event.seq=scope.finish_sequence \
                AND event.event_type='collection.finished' \
               JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
                AND owner.instance_path_nodes='{}' AND owner.instance_path_indices='{}' \
                AND owner.node='lines' \
               JOIN steps ended ON ended.tenant_id=run.tenant_id AND ended.run_id=run.id \
                AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' \
                AND ended.node='done' \
               JOIN scheduler_shards shard ON shard.shard=run_shard(run.id) \
              WHERE run.tenant_id=$1 AND run.id=$2 \
                AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
                AND scope.owner_node='lines'",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("ordinary scheduler's committed Finish identity");
        branch.assert_recorded(&row);

        let scope_id = row.get::<Uuid, _>("scope_id");
        let source_digests = sqlx::query(
            "SELECT iteration_index,result_digest FROM iteration_checkpoints \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 ORDER BY iteration_index",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .bind(scope_id)
        .fetch_all(&flow.api.db.pool)
        .await
        .expect("ordered committed Finish sources")
        .into_iter()
        .map(|source| {
            (
                u32::try_from(source.get::<i32, _>("iteration_index"))
                    .expect("nonnegative checkpoint index"),
                stored_digest(source.get("result_digest")),
            )
        })
        .collect::<Vec<_>>();
        assert_eq!(
            source_digests.len(),
            usize::try_from(row.get::<i32, _>("item_count")).expect("nonnegative item count"),
            "the replay identity names every ordered checkpoint"
        );

        let finish_sequence = u64::try_from(row.get::<i64, _>("finish_sequence"))
            .expect("nonnegative Finish sequence");
        let occurred_at = row.get::<OffsetDateTime, _>("occurred_at");
        let trace_id = row.get::<Option<String>, _>("trace_id");
        let mut finish = CollectionFinish::from_checkpoint_digests(
            CollectionTransition::new(
                flow.tenant,
                NamespaceId::new(row.get("namespace_id")),
                flow.run,
                finish_sequence.checked_sub(1).expect("pre-Finish version"),
                ExecutionInstanceId::root(NodeId::parse("lines").expect("collection owner")),
                SchedulerAuthority::for_run(
                    row.get::<String, _>("scheduler_owner"),
                    flow.run,
                    u64::try_from(row.get::<i64, _>("scheduler_generation"))
                        .expect("nonnegative scheduler generation"),
                ),
                Subject::engine(),
                trace_id.clone(),
                occurred_at,
                None,
            ),
            CollectionScopeId::new(scope_id),
            stored_digest(row.get("result_digest")),
            source_digests.clone(),
            row.get("finish_outbox_id"),
        );
        if let Some(answer) = branch.matching_answer() {
            finish = finish.with_end_answer(answer);
        }
        assert_eq!(
            finish.transition.expected_run_version(),
            finish_sequence - 1,
            "the original command version is the sequence before collection.finished"
        );
        assert_eq!(finish.transition.occurred_at(), occurred_at);
        assert_eq!(finish.transition.trace_id(), trace_id.as_deref());
        assert!(
            finish.result_content().is_none(),
            "metadata-only replay carries no aggregate body"
        );
        assert_eq!(finish.source_digests(), Some(source_digests.as_slice()));
        match branch {
            RecordedEndBranch::EvaluatedExpressionFailure => assert_eq!(
                finish.end_answer(),
                Some(&EndOutputAnswer::ExpressionError),
                "evaluated failure replays its closed recorded answer"
            ),
            RecordedEndBranch::PreEvaluationInstanceLimit => assert!(
                finish.end_answer().is_none(),
                "the recorded pre-evaluation refusal has no End answer"
            ),
        }
        finish
    }

    async fn stable_state(flow: &Flow) -> Value {
        sqlx::query_scalar(
            "SELECT jsonb_build_object( \
                'run',to_jsonb(run), \
                'head',jsonb_build_object('runVersion',run.run_version, \
                  'telemetry',(SELECT telemetry.head_seq FROM telemetry_pending telemetry \
                    WHERE telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id)), \
                'finishReceipt',jsonb_build_object('scopeId',scope.scope_id, \
                  'resultDigest',encode(scope.result_digest,'hex'), \
                  'finishSequence',scope.finish_sequence,'outboxId',scope.finish_outbox_id), \
                'scope',to_jsonb(scope), \
                'steps',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'row',to_jsonb(step),'output',encode(output.content,'hex')) \
                  ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]'::jsonb) \
                  FROM steps step LEFT JOIN payloads output \
                    ON output.tenant_id=step.tenant_id AND output.digest=step.output_digest \
                  WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id), \
                'iterations',(SELECT coalesce(jsonb_agg(to_jsonb(member) \
                  ORDER BY member.iteration_index),'[]'::jsonb) FROM collection_iterations member \
                  WHERE member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                    AND member.scope_id=scope.scope_id), \
                'checkpoints',(SELECT coalesce(jsonb_agg(to_jsonb(checkpoint) \
                  ORDER BY checkpoint.iteration_index),'[]'::jsonb) \
                  FROM iteration_checkpoints checkpoint WHERE checkpoint.tenant_id=scope.tenant_id \
                    AND checkpoint.run_id=scope.run_id AND checkpoint.scope_id=scope.scope_id), \
                'journal',(SELECT coalesce(jsonb_agg(to_jsonb(event) \
                  ORDER BY event.seq),'[]'::jsonb) FROM run_events event \
                  WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id), \
                'tenantPayloads',(SELECT coalesce(jsonb_agg(to_jsonb(payload) \
                  ORDER BY encode(payload.digest,'hex')),'[]'::jsonb) FROM payloads payload \
                  WHERE payload.tenant_id=run.tenant_id), \
                'usage',jsonb_build_object( \
                  'run',(SELECT to_jsonb(usage) FROM run_instance_usage usage \
                    WHERE usage.tenant_id=run.tenant_id AND usage.run_id=run.id), \
                  'retained',(SELECT to_jsonb(retained) FROM tenant_retained_usage retained \
                    WHERE retained.tenant_id=run.tenant_id), \
                  'iterations',(SELECT to_jsonb(iterations) FROM tenant_iteration_usage iterations \
                    WHERE iterations.tenant_id=run.tenant_id)), \
                'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.id),'[]'::jsonb) \
                  FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                    AND audit.resource_type='run' AND audit.resource_id=run.id::text), \
                'outbox',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'id',box.id,'tenantId',box.tenant_id,'aggregateId',box.aggregate_id, \
                    'eventType',box.event_type,'routingKey',box.routing_key, \
                    'payload',box.payload,'createdAt',box.created_at) ORDER BY box.id),'[]'::jsonb) \
                  FROM event_outbox box WHERE box.tenant_id=run.tenant_id \
                    AND box.aggregate_id=run.id)) \
             FROM runs run JOIN iteration_scopes scope \
               ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
              AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
              AND scope.owner_node='lines' \
            WHERE run.tenant_id=$1 AND run.id=$2",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("complete metadata-only replay snapshot")
    }

    async fn assert_original_replay(flow: &Flow, branch: RecordedEndBranch) {
        let finish = original_finish(flow, branch).await;
        let before = stable_state(flow).await;
        // This cold adapter accepts no evaluator or provider, and the reconstructed command carries
        // no aggregate bytes. Replay can therefore validate only retained source/terminal metadata.
        let store = PgCollectionStore::with_definition_cache(
            flow.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let replayed = store
            .finish_collection(&finish)
            .await
            .expect("cold original-identity Finish replay");
        assert!(!replayed.applied);
        assert_eq!(
            replayed.run_version,
            before["head"]["runVersion"]
                .as_u64()
                .expect("persisted nonnegative Run head"),
            "metadata replay reports the retained terminal head"
        );
        assert_eq!(
            stable_state(flow).await,
            before,
            "matching replay changes no payload, usage, journal, head, receipt, audit or outbox state"
        );

        let contradictory = CollectionFinish::from_checkpoint_digests(
            finish.transition.clone(),
            finish.scope_id,
            finish.result_digest,
            finish
                .source_digests()
                .expect("source-bound replay")
                .to_vec(),
            finish.outbox_id,
        )
        .with_end_answer(branch.contradictory_answer());
        assert!(contradictory.result_content().is_none());
        assert_eq!(
            store
                .finish_collection(&contradictory)
                .await
                .expect_err("a contradictory End answer cannot reinterpret recorded history"),
            PrimitiveStoreError::DuplicateMismatch
        );
        assert_eq!(
            stable_state(flow).await,
            before,
            "contradictory replay changes no payload, usage, journal, head, receipt, audit or outbox state"
        );
    }

    pub async fn evaluated_expression_failure(flow: &Flow) {
        assert_original_replay(flow, RecordedEndBranch::EvaluatedExpressionFailure).await;
    }

    pub async fn pre_evaluation_instance_limit(flow: &Flow) {
        assert_original_replay(flow, RecordedEndBranch::PreEvaluationInstanceLimit).await;
    }
}

#[tokio::test]
async fn collection_end_retains_owner_aggregate_and_evaluated_run_result() {
    let name = "collection-end-result";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    // A scalar, distinct from the aggregate, proves the declared expression was evaluated.
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(f.run_status().await, "SUCCEEDED");
    f.verify().await;

    let row = sqlx::query(
        "SELECT scope.lifecycle,aggregate.content AS aggregate,\
         owner_payload.content AS owner_output,end_payload.content AS end_output,\
         run_payload.content AS run_result,\
         owner.output_digest AS owner_digest,scope.result_digest AS aggregate_digest,\
         ended.output_digest AS end_digest,r.result_digest AS run_digest,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.finished') AS finished_events \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
         JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
          AND owner.node='lines' AND owner.instance_path_nodes='{}' \
          AND owner.instance_path_indices='{}' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         JOIN payloads aggregate ON aggregate.tenant_id=r.tenant_id \
          AND aggregate.digest=scope.result_digest \
         LEFT JOIN payloads owner_payload ON owner_payload.tenant_id=r.tenant_id \
          AND owner_payload.digest=owner.output_digest \
         LEFT JOIN payloads end_payload ON end_payload.tenant_id=r.tenant_id \
          AND end_payload.digest=ended.output_digest \
         LEFT JOIN payloads run_payload ON run_payload.tenant_id=r.tenant_id \
          AND run_payload.digest=r.result_digest \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual successful collection and terminal output projection");
    let aggregate = json!([{"amount":17}]);
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(decoded(Some(row.get("aggregate"))), Some(aggregate.clone()));
    assert_eq!(row.get::<i64, _>("finished_events"), 1);
    let outputs = (
        decoded(row.get("owner_output")),
        decoded(row.get("end_output")),
        decoded(row.get("run_result")),
    );
    assert_eq!(
        outputs,
        (Some(aggregate), Some(json!(17)), Some(json!(17))),
        "collection aggregate, evaluated End output and Run result must all be retained"
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        Some(row.get::<Vec<u8>, _>("aggregate_digest"))
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_digest"),
        row.get::<Option<Vec<u8>>, _>("run_digest")
    );
}

#[tokio::test]
async fn collection_success_does_not_bypass_the_successor_end_instance_limit() {
    let name = "collection-end-instance-limit";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["limits"] = json!({"maxInstances":3});
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(
        f.run_status().await,
        "FAILED",
        "End needs its own admitted instance"
    );
    let boundary: (String, String, bool, bool, i64, Option<String>) = sqlx::query_as(
        "SELECT scope.lifecycle,ended.status,ended.output_digest IS NULL,\
         r.result_digest IS NULL,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='end.reached'),\
         (SELECT e.detail->>'code' FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.stop_requested') \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual collection-success/End-refusal boundary");
    assert_eq!(
        boundary,
        (
            "SUCCEEDED".into(),
            "CANCELLED".into(),
            true,
            true,
            0,
            Some("INSTANCE_LIMIT".into()),
        )
    );
    replay::pre_evaluation_instance_limit(&f).await;
    f.verify().await;
}

#[tokio::test]
async fn collection_end_preserves_optional_null_and_typed_evaluation_semantics() {
    struct Case {
        name: &'static str,
        output_expr: Option<&'static str>,
        worker_output: Value,
        expected_status: &'static str,
        expected_end_status: &'static str,
        expected_result: Option<Value>,
        expected_code: Option<&'static str>,
    }

    let ordinary = json!({"amount":17});
    let large_source = Value::String("x".repeat(139_998));
    assert_eq!(large_source.to_string().len(), 140_000);
    assert!(
        Value::Array(vec![large_source.clone()]).to_string().len() <= MAX_PAYLOAD_BYTES,
        "the genuine collection aggregate must fit before End evaluation"
    );
    let cases = [
        Case {
            name: "collection-end-output-omitted",
            output_expr: None,
            worker_output: ordinary.clone(),
            expected_status: "SUCCEEDED",
            expected_end_status: "SUCCEEDED",
            expected_result: None,
            expected_code: None,
        },
        Case {
            name: "collection-end-output-null",
            output_expr: Some("null"),
            worker_output: ordinary.clone(),
            expected_status: "SUCCEEDED",
            expected_end_status: "SUCCEEDED",
            expected_result: Some(Value::Null),
            expected_code: None,
        },
        Case {
            name: "collection-end-output-missing-field",
            output_expr: Some("steps.lines.output[0].missing"),
            worker_output: ordinary,
            expected_status: "FAILED",
            expected_end_status: "FAILED",
            expected_result: None,
            expected_code: Some("EXPRESSION_ERROR"),
        },
        Case {
            name: "collection-end-output-over-value-limit",
            output_expr: Some("[steps.lines.output[0], steps.lines.output[0]]"),
            worker_output: large_source,
            expected_status: "FAILED",
            expected_end_status: "FAILED",
            expected_result: None,
            expected_code: Some("RUN_DATA_LIMIT"),
        },
    ];

    for case in cases {
        let mut document = collection_document(
            case.name,
            vec![
                json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
                json!({"id":"returned","type":"return","result":"steps.work.output"}),
            ],
            1,
        );
        if let Some(expression) = case.output_expr {
            document["nodes"][1]["outputExpr"] = json!(expression);
        }
        let f = Flow::start(case.name, document, json!([{"id":1}])).await;
        f.drive().await;
        let tasks = f.poll().await;
        assert_eq!(tasks.len(), 1, "{} public member lease", case.name);
        f.complete(&tasks[0], case.worker_output.clone()).await;
        f.drive().await;
        assert_eq!(f.run_status().await, case.expected_status, "{}", case.name);

        let row = sqlx::query(
            "SELECT scope.lifecycle,owner.status AS owner_status,ended.status AS end_status, \
                    owner_payload.content AS owner_output,end_payload.content AS end_output, \
                    run_payload.content AS run_result,owner.output_digest AS owner_digest, \
                    scope.result_digest AS aggregate_digest,ended.output_digest AS end_digest, \
                    r.result_digest AS run_digest,r.stop_code, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='end.reached') AS end_events, \
                    (SELECT e.payload_digest FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='end.reached' LIMIT 1) AS end_event_digest, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='run.stop_requested') AS stop_events, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='run.finished') AS finished_events, \
                    (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id \
                      AND o.aggregate_id=r.id AND o.event_type='run.finished') AS finished_outboxes, \
                    COALESCE((SELECT bool_and(o.payload->>'status'=r.status) FROM event_outbox o \
                      WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id \
                        AND o.event_type='run.finished'),false) AS outbox_status_matches \
               FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
                AND scope.run_id=r.id AND scope.owner_node='lines' \
                AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
               JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
                AND owner.node='lines' AND owner.instance_path_nodes='{}' \
                AND owner.instance_path_indices='{}' \
               JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
                AND ended.node='done' AND ended.instance_path_nodes='{}' \
                AND ended.instance_path_indices='{}' \
               LEFT JOIN payloads owner_payload ON owner_payload.tenant_id=r.tenant_id \
                AND owner_payload.digest=owner.output_digest \
               LEFT JOIN payloads end_payload ON end_payload.tenant_id=r.tenant_id \
                AND end_payload.digest=ended.output_digest \
               LEFT JOIN payloads run_payload ON run_payload.tenant_id=r.tenant_id \
                AND run_payload.digest=r.result_digest \
              WHERE r.tenant_id=$1 AND r.id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("public collection End semantic projection");

        let aggregate = Value::Array(vec![case.worker_output]);
        assert_eq!(
            row.get::<String, _>("lifecycle"),
            "SUCCEEDED",
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<String, _>("owner_status"),
            "SUCCEEDED",
            "{}",
            case.name
        );
        assert_eq!(
            decoded(row.get("owner_output")),
            Some(aggregate),
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<Option<Vec<u8>>, _>("owner_digest"),
            row.get::<Option<Vec<u8>>, _>("aggregate_digest"),
            "{} owner output is the successful scope aggregate",
            case.name,
        );
        assert_eq!(
            row.get::<String, _>("end_status"),
            case.expected_end_status,
            "{}",
            case.name
        );
        assert_eq!(
            decoded(row.get("end_output")),
            case.expected_result.clone(),
            "{}",
            case.name,
        );
        assert_eq!(
            decoded(row.get("run_result")),
            case.expected_result.clone(),
            "{}",
            case.name,
        );
        assert_eq!(
            row.get::<Option<String>, _>("stop_code").as_deref(),
            case.expected_code,
            "{}",
            case.name
        );

        let expected_terminal_event: i64 = if case.expected_code.is_none() { 1 } else { 0 };
        assert_eq!(
            row.get::<i64, _>("end_events"),
            expected_terminal_event,
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<i64, _>("stop_events"),
            1 - expected_terminal_event,
            "{}",
            case.name
        );
        assert_eq!(row.get::<i64, _>("finished_events"), 1, "{}", case.name);
        assert_eq!(row.get::<i64, _>("finished_outboxes"), 1, "{}", case.name);
        assert!(row.get::<bool, _>("outbox_status_matches"), "{}", case.name);

        let end_digest = row.get::<Option<Vec<u8>>, _>("end_digest");
        let run_digest = row.get::<Option<Vec<u8>>, _>("run_digest");
        let event_digest = row.get::<Option<Vec<u8>>, _>("end_event_digest");
        if case.expected_result.is_some() {
            assert!(
                end_digest.is_some(),
                "{} retained explicit result",
                case.name
            );
            assert_eq!(end_digest, run_digest, "{} End and Run digest", case.name);
            assert_eq!(
                end_digest, event_digest,
                "{} EndReached payload digest",
                case.name
            );
        } else {
            assert_eq!(
                (end_digest, run_digest, event_digest),
                (None, None, None),
                "{}",
                case.name
            );
        }
        if case.expected_code == Some("EXPRESSION_ERROR") {
            replay::evaluated_expression_failure(&f).await;
        }
        f.verify().await;
    }
}

#[tokio::test]
async fn collection_result_uses_pinned_output_schema_for_http_disclosure() {
    const PUBLIC_ANSWER: &str = "collection-public-answer-7f4c";
    const INTERNAL_MARKER: &str = "collection-internal-marker-7f4c";
    const PII_MARKER: &str = "collection-pii-marker-7f4c";
    const UNMARKED_KEY: &str = "collection-unmarked-key-7f4c";
    const UNMARKED_MARKER: &str = "collection-unmarked-marker-7f4c";
    const SECRET_MARKER: &str = "collection-secret-marker-7f4c";

    let name = "collection-result-classification";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0]");
    document["outputSchema"] = json!({
        "type":"object",
        "additionalProperties":true,
        "x-sagat-classification":"public",
        "properties":{
            "publicAnswer":{"type":"string","x-sagat-classification":"public"},
            "internalMarker":{"type":"string","x-sagat-classification":"internal"},
            "piiMarker":{"type":"string","x-sagat-classification":"pii"},
            "secretMarker":{"type":"string","x-sagat-classification":"secret"}
        }
    });
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    let (_log_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);

    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(
        &tasks[0],
        json!({
            "publicAnswer":PUBLIC_ANSWER,
            "internalMarker":INTERNAL_MARKER,
            "piiMarker":PII_MARKER,
            (UNMARKED_KEY):UNMARKED_MARKER,
            "secretMarker":SECRET_MARKER
        }),
    )
    .await;
    f.drive().await;
    assert_eq!(f.run_status().await, "SUCCEEDED");

    let ordinary = f
        .api
        .as_user(
            reqwest::Method::GET,
            &format!("/runs/{}", f.run),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("read ordinary run resource");
    assert_eq!(ordinary.status(), reqwest::StatusCode::OK);
    let ordinary_wire = ordinary.text().await.expect("ordinary run body");
    for marker in [
        PUBLIC_ANSWER,
        INTERNAL_MARKER,
        PII_MARKER,
        UNMARKED_KEY,
        UNMARKED_MARKER,
        SECRET_MARKER,
    ] {
        assert!(
            !ordinary_wire.contains(marker),
            "ordinary run response exposed collection result marker {marker}"
        );
    }

    let response = f
        .api
        .as_user(
            reqwest::Method::POST,
            &format!("/runs/{}/payload-disclosures", f.run),
            "alice",
            "acme",
        )
        .json(&json!({"source":"result"}))
        .send()
        .await
        .expect("disclose actual collection Run result");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    let disclosure_wire = response.text().await.expect("result disclosure body");
    let disclosure: Value =
        serde_json::from_str(&disclosure_wire).expect("classified result response");
    assert_eq!(disclosure["selection"]["source"], "result");
    assert_eq!(disclosure["policy"], "classified-v1");
    assert_eq!(disclosure["caller"]["revealPii"], "not_granted");
    assert_eq!(disclosure["caller"]["revealed"], false);
    assert_eq!(disclosure["kind"], "disclosed");
    assert_eq!(disclosure["schema"]["carrier"], "outputSchema");
    assert_eq!(disclosure["schema"]["versionNumber"], 1);
    assert_eq!(disclosure["schema"]["rules"], 1);

    let children = disclosure["tree"]["children"]
        .as_array()
        .expect("classified result children");
    let child = |name: &str| {
        children
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("classified result child {name}"))
    };
    assert_eq!(child("publicAnswer")["node"]["kind"], "value");
    assert_eq!(child("publicAnswer")["node"]["level"], "public");
    assert_eq!(child("publicAnswer")["node"]["value"], PUBLIC_ANSWER);
    assert_eq!(child("internalMarker")["node"]["kind"], "value");
    assert_eq!(child("internalMarker")["node"]["value"], INTERNAL_MARKER);
    assert_eq!(child("piiMarker")["node"]["kind"], "withheld");
    assert_eq!(child("piiMarker")["node"]["reason"], "LEVEL_NOT_PERMITTED");
    assert_eq!(child("secretMarker")["node"]["kind"], "withheld");
    assert_eq!(child("secretMarker")["node"]["reason"], "SECRET");
    for marker in [PII_MARKER, UNMARKED_KEY, UNMARKED_MARKER, SECRET_MARKER] {
        assert!(
            !disclosure_wire.contains(marker),
            "classified result response exposed private marker {marker}"
        );
    }

    let audit = f
        .api
        .as_user(
            reqwest::Method::GET,
            &format!(
                "/audit?action=payload.disclosed&resourceId={}&actor=alice&limit=20",
                f.run
            ),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("read disclosure audit through current tenant authorization");
    assert_eq!(audit.status(), reqwest::StatusCode::OK);
    let audit_wire = audit.text().await.expect("audit response body");
    let audit: Value = serde_json::from_str(&audit_wire).expect("audit page");
    let record = audit["items"]
        .as_array()
        .expect("audit items")
        .iter()
        .find(|record| {
            record["action"] == "payload.disclosed"
                && record["resourceId"] == f.run.to_string()
                && record["reason"] == "classified-v1:result"
        })
        .expect("classified collection result disclosure audit");
    assert_eq!(record["actor"], "alice");
    assert_eq!(record["resourceType"], "run");
    assert!(
        record["traceId"]
            .as_str()
            .is_some_and(|trace| !trace.is_empty()),
        "disclosure audit retains trace correlation"
    );
    for marker in [
        PUBLIC_ANSWER,
        "publicAnswer",
        "internalMarker",
        "piiMarker",
        "secretMarker",
        INTERNAL_MARKER,
        PII_MARKER,
        UNMARKED_KEY,
        UNMARKED_MARKER,
        SECRET_MARKER,
    ] {
        assert!(
            !audit_wire.contains(marker),
            "audit response exposed payload marker {marker}"
        );
        assert!(
            !logs.text().contains(marker),
            "structured logs exposed payload marker {marker}"
        );
    }
    f.verify().await;
}

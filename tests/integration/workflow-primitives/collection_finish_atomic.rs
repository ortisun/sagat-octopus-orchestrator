//! The successful root collection finish is one atomic native End transaction.

use sagat_application::{
    execution::{DefinitionCache, EndOutputAnswer},
    primitives::{
        CollectionFinish, CollectionScopeId, CollectionStore, CollectionTransition,
        PrimitiveStoreError, SchedulerAuthority,
    },
};
use sagat_domain::{
    execution::{ExecutionInstanceId, RunId},
    identity::{NamespaceId, Subject},
    workflow::{Digest, NodeId},
};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::PgCollectionStore;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support::scope_flow::{Flow, collection_document};
use crate::support::{eventually, logs};

fn positive_advisory_key() -> i64 {
    i64::try_from(Uuid::new_v4().as_u128() & ((1_u128 << 63) - 1)).expect("positive advisory key")
}

fn stored_digest(bytes: Vec<u8>) -> Digest {
    Digest::new(bytes.try_into().expect("stored SHA-256 digest"))
}

async fn install_finish_fault(pool: &PgPool, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION collection_finish_atomic_gate() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM pg_advisory_xact_lock({key}); \
         RAISE EXCEPTION 'test-only collection finish abort' \
           USING ERRCODE='23514',CONSTRAINT='collection_finish_atomic_forced_abort'; \
         END $$; \
         CREATE TRIGGER collection_finish_atomic_outbox_gate \
         AFTER INSERT ON event_outbox FOR EACH ROW \
         WHEN (NEW.event_type='collection.finished') \
         EXECUTE FUNCTION collection_finish_atomic_gate();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install labelled collection Finish fault gate");
}

async fn remove_finish_fault(pool: &PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER collection_finish_atomic_outbox_gate ON event_outbox; \
         DROP FUNCTION collection_finish_atomic_gate();",
    )
    .execute(pool)
    .await
    .expect("remove labelled collection Finish fault gate");
}

async fn snapshot(pool: &PgPool, tenant: Uuid, run: RunId) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'run',to_jsonb(r), \
            'runResult',encode(run_result.content,'hex'), \
            'scope',to_jsonb(scope), \
            'scopeResult',encode(scope_result.content,'hex'), \
            'steps',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                'row',to_jsonb(step),'output',encode(output.content,'hex')) \
              ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]'::jsonb) \
              FROM steps step LEFT JOIN payloads output \
                ON output.tenant_id=step.tenant_id AND output.digest=step.output_digest \
              WHERE step.tenant_id=r.tenant_id AND step.run_id=r.id), \
            'iterations',(SELECT coalesce(jsonb_agg(to_jsonb(member) \
              ORDER BY member.iteration_index),'[]'::jsonb) \
              FROM collection_iterations member \
              WHERE member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                AND member.scope_id=scope.scope_id), \
            'checkpoints',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                'row',to_jsonb(checkpoint),'result',encode(result.content,'hex')) \
              ORDER BY checkpoint.iteration_index),'[]'::jsonb) \
              FROM iteration_checkpoints checkpoint JOIN payloads result \
                ON result.tenant_id=checkpoint.tenant_id \
               AND result.digest=checkpoint.result_digest \
              WHERE checkpoint.tenant_id=scope.tenant_id \
                AND checkpoint.run_id=scope.run_id AND checkpoint.scope_id=scope.scope_id), \
            'journal',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                'row',to_jsonb(event),'payload',encode(payload.content,'hex')) \
              ORDER BY event.seq),'[]'::jsonb) \
              FROM run_events event LEFT JOIN payloads payload \
                ON payload.tenant_id=event.tenant_id AND payload.digest=event.payload_digest \
              WHERE event.tenant_id=r.tenant_id AND event.run_id=r.id), \
            'tenantPayloads',(SELECT coalesce(jsonb_agg(to_jsonb(payload) \
              ORDER BY encode(payload.digest,'hex')),'[]'::jsonb) \
              FROM payloads payload WHERE payload.tenant_id=r.tenant_id), \
            'tenantRetainedUsage',(SELECT to_jsonb(retained) \
              FROM tenant_retained_usage retained WHERE retained.tenant_id=r.tenant_id), \
            'usage',(SELECT to_jsonb(usage) FROM run_instance_usage usage \
              WHERE usage.tenant_id=r.tenant_id AND usage.run_id=r.id), \
            'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.id),'[]'::jsonb) \
              FROM audit_records audit WHERE audit.tenant_id=r.tenant_id \
                AND audit.resource_type='run' AND audit.resource_id=r.id::text), \
            'outbox',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                'id',box.id,'tenantId',box.tenant_id,'aggregateId',box.aggregate_id, \
                'eventType',box.event_type,'routingKey',box.routing_key,'payload',box.payload, \
                'createdAt',box.created_at) \
              ORDER BY box.id),'[]'::jsonb) FROM event_outbox box \
              WHERE box.tenant_id=r.tenant_id AND box.aggregate_id=r.id), \
            'telemetry',(SELECT jsonb_build_object('head',telemetry.head_seq) \
              FROM telemetry_pending telemetry \
              WHERE telemetry.tenant_id=r.tenant_id AND telemetry.run_id=r.id)) \
         FROM runs r JOIN iteration_scopes scope \
           ON scope.tenant_id=r.tenant_id AND scope.run_id=r.id \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
          AND scope.owner_node='lines' \
         LEFT JOIN payloads run_result \
           ON run_result.tenant_id=r.tenant_id AND run_result.digest=r.result_digest \
         LEFT JOIN payloads scope_result \
           ON scope_result.tenant_id=scope.tenant_id AND scope_result.digest=scope.result_digest \
        WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("atomic collection Finish snapshot")
}

async fn drive_to_real_checkpoint(flow: &Flow) {
    for _ in 0..8 {
        let ready: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM iteration_scopes scope \
               WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
                 AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
                 AND scope.owner_node='lines' AND scope.lifecycle='OPEN' \
                 AND scope.item_count=1 AND scope.next_admission_index=1 \
                 AND scope.admitted_count=1 AND scope.completed_count=1 \
                 AND scope.active_count=0 AND scope.result_digest IS NULL \
                 AND scope.finish_sequence IS NULL AND scope.finish_outbox_id IS NULL \
                 AND EXISTS(SELECT 1 FROM collection_iterations member \
                   JOIN iteration_checkpoints checkpoint \
                     ON checkpoint.tenant_id=member.tenant_id \
                    AND checkpoint.run_id=member.run_id AND checkpoint.scope_id=member.scope_id \
                    AND checkpoint.iteration_index=member.iteration_index \
                   WHERE member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                     AND member.scope_id=scope.scope_id AND member.iteration_index=0 \
                     AND member.lifecycle='CHECKPOINTED' AND NOT member.slot_held \
                     AND member.return_instance_id=checkpoint.return_instance_id \
                     AND member.result_digest=checkpoint.result_digest \
                     AND member.checkpoint_sequence=checkpoint.journal_sequence))",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("real checkpoint boundary");
        if ready {
            return;
        }
        let report = flow.scheduler.tick().await;
        assert_eq!(report.failed, 0, "checkpoint action failed: {report:?}");
    }
    panic!("public member did not reach its real checkpoint boundary");
}

async fn waiting_finish_insert(flow: &Flow, gate_pid: i32) -> i32 {
    eventually(
        "actual collection.finished insert waits inside the post-persist fault trigger",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
                   WHERE datname=current_database() AND usename='sagat_app' \
                     AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                     AND position('INSERT INTO event_outbox' in query)>0 LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&flow.api.db.pool)
            .await
            .expect("collection Finish advisory wait witness")
        },
    )
    .await
}

async fn assert_success(flow: &Flow) {
    let row = sqlx::query(
        "SELECT scope.lifecycle,r.status,aggregate.content AS aggregate, \
                owner_output.content AS owner_output,end_output.content AS end_output, \
                run_result.content AS run_result,scope.result_digest AS scope_digest, \
                owner.output_digest AS owner_digest,ended.output_digest AS end_digest, \
                r.result_digest AS run_digest,usage.logical_output_bytes, \
                (SELECT coalesce(sum(octet_length(output.content)),0)::bigint \
                  FROM steps step JOIN payloads output \
                    ON output.tenant_id=step.tenant_id AND output.digest=step.output_digest \
                  WHERE step.tenant_id=r.tenant_id AND step.run_id=r.id) AS step_output_bytes, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=r.tenant_id \
                  AND event.run_id=r.id AND event.event_type='collection.finished') AS finish_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=r.tenant_id \
                  AND event.run_id=r.id AND event.event_type='end.reached') AS end_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=r.tenant_id \
                  AND event.run_id=r.id AND event.event_type='run.finished') AS run_events, \
                (SELECT count(*) FROM event_outbox box WHERE box.tenant_id=r.tenant_id \
                  AND box.aggregate_id=r.id AND box.event_type='collection.finished') AS finish_outboxes, \
                (SELECT count(*) FROM event_outbox box WHERE box.tenant_id=r.tenant_id \
                  AND box.aggregate_id=r.id AND box.event_type='run.finished') AS run_outboxes, \
                scope.finish_sequence,telemetry.head_seq,r.run_version \
           FROM runs r JOIN iteration_scopes scope \
             ON scope.tenant_id=r.tenant_id AND scope.run_id=r.id \
            AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
            AND scope.owner_node='lines' \
           JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
            AND owner.instance_path_nodes='{}' AND owner.instance_path_indices='{}' \
            AND owner.node='lines' \
           JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
            AND ended.instance_path_nodes='{}' AND ended.instance_path_indices='{}' \
            AND ended.node='done' \
           JOIN payloads aggregate \
             ON aggregate.tenant_id=scope.tenant_id AND aggregate.digest=scope.result_digest \
           JOIN payloads owner_output \
             ON owner_output.tenant_id=owner.tenant_id AND owner_output.digest=owner.output_digest \
           JOIN payloads end_output \
             ON end_output.tenant_id=ended.tenant_id AND end_output.digest=ended.output_digest \
           JOIN payloads run_result \
             ON run_result.tenant_id=r.tenant_id AND run_result.digest=r.result_digest \
           JOIN run_instance_usage usage \
             ON usage.tenant_id=r.tenant_id AND usage.run_id=r.id \
           JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=r.tenant_id AND telemetry.run_id=r.id \
          WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("committed collection Finish projection");
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(row.get::<String, _>("status"), "SUCCEEDED");
    assert_eq!(row.get::<Vec<u8>, _>("aggregate"), br#"[{"amount":17}]"#);
    assert_eq!(row.get::<Vec<u8>, _>("owner_output"), br#"[{"amount":17}]"#);
    assert_eq!(row.get::<Vec<u8>, _>("end_output"), b"17");
    assert_eq!(row.get::<Vec<u8>, _>("run_result"), b"17");
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        row.get::<Option<Vec<u8>>, _>("scope_digest")
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_digest"),
        row.get::<Option<Vec<u8>>, _>("run_digest")
    );
    assert_eq!(
        row.get::<i64, _>("logical_output_bytes"),
        row.get::<i64, _>("step_output_bytes")
    );
    assert_eq!(row.get::<i64, _>("logical_output_bytes"), 30);
    assert_eq!(
        (
            row.get::<i64, _>("finish_events"),
            row.get::<i64, _>("end_events"),
            row.get::<i64, _>("run_events"),
            row.get::<i64, _>("finish_outboxes"),
            row.get::<i64, _>("run_outboxes"),
        ),
        (1, 1, 1, 1, 1)
    );
    assert!(row.get::<i64, _>("finish_sequence") < row.get::<i64, _>("run_version"));
    assert_eq!(
        row.get::<i64, _>("head_seq"),
        row.get::<i64, _>("run_version")
    );
}

async fn source_bound_cold_replay(flow: &Flow) -> CollectionFinish {
    let row = sqlx::query(
        "SELECT run.namespace_id,run.trace_id,scope.scope_id,scope.result_digest, \
                scope.finish_outbox_id,scope.finish_sequence,event.occurred_at, \
                shard.owner AS scheduler_owner,shard.generation AS scheduler_generation \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN run_events event \
             ON event.tenant_id=scope.tenant_id AND event.run_id=scope.run_id \
            AND event.seq=scope.finish_sequence AND event.event_type='collection.finished' \
           JOIN scheduler_shards shard ON shard.shard=run_shard(run.id) \
          WHERE run.tenant_id=$1 AND run.id=$2 \
            AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
            AND scope.owner_node='lines'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("committed Finish replay identity");
    let source_digests = sqlx::query(
        "SELECT iteration_index,result_digest FROM iteration_checkpoints \
          WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 ORDER BY iteration_index",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .bind(row.get::<Uuid, _>("scope_id"))
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("ordered committed checkpoint identities")
    .into_iter()
    .map(|source| {
        (
            u32::try_from(source.get::<i32, _>("iteration_index"))
                .expect("nonnegative checkpoint index"),
            stored_digest(source.get("result_digest")),
        )
    })
    .collect::<Vec<_>>();
    assert_eq!(source_digests.len(), 1, "the public flow has one source");
    let finish_sequence =
        u64::try_from(row.get::<i64, _>("finish_sequence")).expect("nonnegative Finish sequence");
    let replay = CollectionFinish::from_checkpoint_digests(
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
            row.get("trace_id"),
            row.get::<OffsetDateTime, _>("occurred_at"),
            None,
        ),
        CollectionScopeId::new(row.get("scope_id")),
        stored_digest(row.get("result_digest")),
        source_digests.clone(),
        row.get("finish_outbox_id"),
    )
    .with_end_answer(EndOutputAnswer::Succeeded(b"17".to_vec()));
    assert_eq!(
        replay.result_content(),
        None,
        "cold replay carries no aggregate body"
    );
    assert_eq!(
        replay.source_digests(),
        Some(source_digests.as_slice()),
        "cold replay preserves the exact ordered source identity"
    );
    replay
}

#[tokio::test(flavor = "current_thread")]
async fn collection_finish_is_atomic_after_persistence_and_retry_commits_once() {
    let name = "collection-finish-atomic";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let flow = Flow::start(name, document, json!([{"id":1}])).await;
    flow.drive().await;
    let tasks = flow.poll().await;
    assert_eq!(tasks.len(), 1, "one genuine public member lease");
    flow.complete(&tasks[0], json!({"amount":17})).await;
    drive_to_real_checkpoint(&flow).await;
    let (_fault_log_guard, fault_logs) = logs::capture_at(tracing::Level::ERROR);

    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    let key = positive_advisory_key();
    install_finish_fault(&flow.api.db.pool, key).await;
    let mut gate = flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned Finish advisory gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("Finish advisory owner pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *gate)
        .await
        .expect("hold Finish advisory gate");

    let finish = flow.scheduler.tick();
    tokio::pin!(finish);
    let observe = async {
        let blocked_pid = waiting_finish_insert(&flow, gate_pid).await;
        assert_ne!(blocked_pid, gate_pid);
        let blocked = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        assert_eq!(
            blocked, before,
            "another connection cannot observe owner/End/Run/result/journal/ledger/scope/audit/outbox/telemetry/payload/accounting writes"
        );
        let still_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
               WHERE pid=$1 AND state='active' AND wait_event_type='Lock' \
                 AND $2=ANY(pg_blocking_pids(pid)) \
                 AND position('INSERT INTO event_outbox' in query)>0)",
        )
        .bind(blocked_pid)
        .bind(gate_pid)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("Finish writer remains behind the owned gate after observation");
        gate.rollback()
            .await
            .expect("release only the owned Finish advisory gate");
        still_blocked
    };
    tokio::pin!(observe);
    let (failed, blocked_through_observation) = tokio::join!(&mut finish, &mut observe);
    assert!(
        blocked_through_observation,
        "the actual writer must remain blocked until the observation completes and releases the gate"
    );
    assert_eq!(
        failed.failed, 1,
        "the injected post-persist Finish fault is observed"
    );
    let fault_log = fault_logs.text();
    assert!(
        fault_log.contains("test-only collection finish abort"),
        "the failed scheduler action must be the labelled fault, not a timeout: {fault_log}"
    );
    assert!(
        !fault_log.contains("statement timeout"),
        "the writer timed out before the labelled fault: {fault_log}"
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        before,
        "the injected outbox failure rolls the whole Finish transaction back"
    );

    remove_finish_fault(&flow.api.db.pool).await;
    let retry = flow.scheduler.tick().await;
    assert_eq!(
        retry.failed, 0,
        "ordinary scheduler retry succeeds: {retry:?}"
    );
    assert_eq!(
        retry.collections_advanced, 1,
        "ordinary scheduler owns the Finish retry"
    );
    assert_success(&flow).await;
    flow.verify().await;

    let stable = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    let cold_replay = source_bound_cold_replay(&flow).await;
    let cold_store = PgCollectionStore::with_definition_cache(
        flow.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let replayed = cold_store
        .finish_collection(&cold_replay)
        .await
        .expect("source-bound metadata-only Finish replay");
    assert!(!replayed.applied);
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        stable,
        "matching metadata-only replay changes no persisted or accounting state"
    );

    let mut wrong_sources = cold_replay
        .source_digests()
        .expect("source-bound replay")
        .to_vec();
    wrong_sources[0].1 = Digest::new([0xa5; 32]);
    let contradictory = CollectionFinish::from_checkpoint_digests(
        cold_replay.transition.clone(),
        cold_replay.scope_id,
        cold_replay.result_digest,
        wrong_sources,
        cold_replay.outbox_id,
    )
    .with_end_answer(EndOutputAnswer::Succeeded(b"17".to_vec()));
    assert_eq!(
        cold_store
            .finish_collection(&contradictory)
            .await
            .expect_err("checkpoint source identity is validated by the store"),
        PrimitiveStoreError::DuplicateMismatch
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        stable,
        "contradictory source metadata changes no persisted or accounting state"
    );
    let contradictory_end = cold_replay
        .clone()
        .with_end_answer(EndOutputAnswer::Succeeded(b"18".to_vec()));
    assert_eq!(
        cold_store
            .finish_collection(&contradictory_end)
            .await
            .expect_err("retained End answer is part of exact replay identity"),
        PrimitiveStoreError::DuplicateMismatch
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        stable,
        "contradictory End bytes change no persisted or accounting state"
    );
    flow.verify().await;
}

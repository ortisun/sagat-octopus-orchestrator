//! Dormant native source certification from real HTTP/gRPC execution. No production caller.

use crate::support::scope_flow::{Flow, collection_document};
use sagat_application::{
    execution::DefinitionCache,
    leaf_certification::{
        CertifyLeafCollectionSuccess, LeafCollectionPathIdentity, LeafCollectionSuccessError,
        LeafCollectionSuccessStore,
    },
    primitives::SchedulerAuthority,
    shards::ShardStore,
};
use sagat_domain::{
    execution::InstancePath, identity::NamespaceId, sharding::Shard, workflow::NodeId,
};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::PgLeafCollectionSuccessStore;
use serde_json::{Value, json};
use sqlx::Row;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

#[path = "leaf_certification_binding.rs"]
mod binding;
#[path = "leaf_certification_closure.rs"]
mod closure;
#[path = "leaf_certification_commit_loss.rs"]
mod commit_loss;
use crate::capacity_finish::commit_proxy;
#[path = "leaf_certification_late_reports.rs"]
mod late_reports;
#[path = "leaf_certification_later_work.rs"]
mod later_work;
#[path = "leaf_certification_observations.rs"]
mod observations;
#[path = "leaf_certification_process.rs"]
mod process;
#[path = "leaf_certification_refusals.rs"]
mod refusals;
#[path = "leaf_certification_saga.rs"]
mod saga;
#[path = "scoped_leaf_finish_native.rs"]
mod scoped_finish;

fn document(name: &str) -> Value {
    let mut d = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    d["nodes"][0]["next"] = json!("later");
    d["nodes"].as_array_mut().expect("root nodes").insert(1,json!({"id":"later","type":"forEach","items":"input.lines","body":"later_line","result":"bodyResult","maxItems":8,"maxConcurrency":1,"next":"done"}));
    d["bodies"]["later_line"] = json!({"entrypoint":"work","nodes":[
        {"id":"work","type":"activity","activity":"line.charge","next":"returned"},
        {"id":"returned","type":"return","result":"steps.work.output"}
    ]});
    d
}

async fn command(f: &Flow) -> CertifyLeafCollectionSuccess {
    let row = sqlx::query("SELECT namespace_id,run_version FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("own run discovery");
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=$1")
            .bind(i16::from(Shard::of(f.run.as_uuid()).index()))
            .fetch_one(&f.api.db.pool)
            .await
            .expect("current scheduler authority discovery");
    CertifyLeafCollectionSuccess {
        identity: LeafCollectionPathIdentity::new(
            f.tenant,
            NamespaceId::new(row.get::<Uuid, _>("namespace_id")),
            f.run,
            InstancePath::root()
                .child(NodeId::parse("lines").expect("owner"), 0)
                .expect("path"),
        )
        .expect("depth1"),
        expected_run_revision: u64::try_from(row.get::<i64, _>("run_version")).expect("revision"),
        scheduler: SchedulerAuthority::for_run(
            owner,
            f.run,
            u64::try_from(generation).expect("generation"),
        ),
    }
}

async fn selected_sources(f: &Flow) -> String {
    // Full selected-source fingerprint is a test oracle, never a production certification query.
    sqlx::query_scalar("SELECT md5(jsonb_build_object('scope',(SELECT to_jsonb(s) FROM iteration_scopes s WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines'),'iteration',(SELECT to_jsonb(i) FROM collection_iterations i JOIN iteration_scopes s USING(tenant_id,run_id,scope_id) WHERE i.tenant_id=$1 AND i.run_id=$2 AND s.owner_node='lines'),'checkpoint',(SELECT to_jsonb(c) FROM iteration_checkpoints c JOIN iteration_scopes s USING(tenant_id,run_id,scope_id) WHERE c.tenant_id=$1 AND c.run_id=$2 AND s.owner_node='lines'),'owner',(SELECT to_jsonb(t) FROM steps t WHERE tenant_id=$1 AND run_id=$2 AND node='lines' AND instance_path_nodes='{}' AND instance_path_indices='{}'),'saga',(SELECT to_jsonb(s) FROM scope_sagas s WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines' AND owner_path_nodes='{}' AND owner_path_indices='{}'),'saga_entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY to_jsonb(e)::text) FROM scope_saga_entries e WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines' AND owner_path_nodes='{}' AND owner_path_indices='{}' AND iteration_index=0),'steps',(SELECT jsonb_agg(to_jsonb(t) ORDER BY node) FROM steps t WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::integer[]))::text)")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("selected source oracle")
}

#[tokio::test]
async fn native_certification_compares_far_future_scheduler_expiry_on_the_server() {
    let f = Flow::start(
        "leaf-scheduler-time",
        document("leaf-scheduler-time"),
        json!([{"v":1}]),
    )
    .await;
    f.drive().await;
    let c = command(&f).await;
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::SourceNotClosed)
    );
    let lease = c.scheduler.lease();
    let renewed = f
        .api
        .executions
        .renew(
            c.scheduler.owner(),
            &[lease],
            Duration::from_secs(10_000 * 366 * 86_400),
        )
        .await
        .expect("actual lease renewal accepts large Duration");
    assert_eq!(renewed, vec![lease]);
    let beyond: bool = sqlx::query_scalar("SELECT expires_at>=TIMESTAMPTZ '10000-01-01 00:00:00+00' FROM scheduler_shards WHERE shard=$1")
        .bind(i16::from(lease.shard.index())).fetch_one(&f.api.db.pool).await.expect("finite expiry range metadata");
    assert!(beyond);
    assert_eq!(
        store.certify_leaf_collection_success(&c).await,
        Err(LeafCollectionSuccessError::SourceNotClosed)
    );
}

#[tokio::test]
async fn native_certification_refuses_unrepresentable_canonical_time() {
    let f = Flow::start(
        "leaf-canonical-time",
        document("leaf-canonical-time"),
        json!([{"v":1}]),
    )
    .await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"accepted":true})).await;
    f.drive().await;
    let mut c = command(&f).await;
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    for (index, date, representable) in [
        (0, "9999-12-31 23:59:59.999999+00", true),
        (1, "10000-01-01 00:00:00+00", false),
        (2, "294000-01-01 00:00:00+00", false),
        (3, "1000-01-01 00:00:00+00 BC", true),
    ] {
        // Owner fixtures test canonical decoding only. They are deliberately not source proof.
        sqlx::query("INSERT INTO leaf_collection_success_certifications(tenant_id,namespace_id,run_id,owner_node,iteration_index,profile,definition_version_id,scope_id,body_node,return_node,admitted_sequence,returned_sequence,checkpoint_sequence,checkpoint_result_digest,finished_sequence,aggregate_result_digest,owner_routed_sequence,certified_run_revision,certified_at,audit_id) SELECT tenant_id,namespace_id,id,'lines',$3,'ORDINARY_LEAF_V1',workflow_version_id,gen_random_uuid(),'line','returned',1,2,3,decode(repeat('11',32),'hex'),4,decode(repeat('22',32),'hex'),5,run_version,CAST($4 AS text)::timestamptz,gen_random_uuid() FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).bind(index).bind(date)
            .execute(&f.api.db.pool).await.expect("finite timestamp accepted by schema");
        c.identity = LeafCollectionPathIdentity::new(
            c.identity.tenant_id(),
            c.identity.namespace_id(),
            c.identity.run_id(),
            InstancePath::root()
                .child(
                    NodeId::parse("lines").expect("owner"),
                    u32::try_from(index).expect("index"),
                )
                .expect("path"),
        )
        .expect("qualified fixture identity");
        let result = store.certify_leaf_collection_success(&c).await;
        if representable {
            assert!(result.expect("representable upper bound").replayed);
        } else {
            assert_eq!(result, Err(LeafCollectionSuccessError::ContradictorySource));
        }
    }
}

#[tokio::test]
async fn native_certification_guard_failure_redacts_driver_diagnostics() {
    let f = Flow::start(
        "leaf-guard-log",
        document("leaf-guard-log"),
        json!([{"v":1}]),
    )
    .await;
    f.drive().await;
    let c = command(&f).await;
    let pool = f.api.db.runtime_pool_with(1).await;
    // Shadow only this disposable connection's clock lookup; other fixture pools keep pg_catalog
    // first. The real scheduler guard must handle an actual PostgreSQL diagnostic safely.
    sqlx::raw_sql("CREATE FUNCTION public.clock_timestamp() RETURNS timestamptz LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'LEAF_GUARD_DIAGNOSTIC_CANARY' USING ERRCODE='23514'; END $$;")
        .execute(&f.api.db.pool).await.expect("clock diagnostic injection");
    sqlx::query("SET search_path=public,pg_catalog")
        .execute(&pool)
        .await
        .expect("isolated clock shadow");
    let store = PgLeafCollectionSuccessStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let (_guard, captured) = crate::support::logs::capture();
    tracing::warn!("leaf guard capture positive control");
    let error = store
        .certify_leaf_collection_success(&c)
        .await
        .expect_err("guard fault");
    assert!(matches!(error, LeafCollectionSuccessError::Storage(_)));
    if let LeafCollectionSuccessError::Storage(sagat_application::error::StoreError::Internal {
        detail,
    }) = &error
    {
        assert!(detail.contains("23514"));
        assert!(!detail.contains("LEAF_GUARD_DIAGNOSTIC_CANARY"));
    } else {
        panic!("expected sanitized internal diagnostic");
    }
    assert!(!format!("{error} {error:?}").contains("LEAF_GUARD_DIAGNOSTIC_CANARY"));
    let logs = captured.text();
    assert!(logs.contains("leaf guard capture positive control"));
    assert!(
        !logs.contains("LEAF_GUARD_DIAGNOSTIC_CANARY"),
        "raw guard diagnostic leaked"
    );
    pool.close().await;
    sqlx::query("DROP FUNCTION public.clock_timestamp()")
        .execute(&f.api.db.pool)
        .await
        .expect("remove diagnostic injection");
}

#[tokio::test]
async fn native_certification_is_metadata_only_and_replays_after_later_scope_failure() {
    let f = Flow::start("leaf-native", document("leaf-native"), json!([{"v":1}])).await;
    // Explicit test-only activation. Released schema stays SELECT-only until the writer's gates.
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("dormant native test capability");
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&f.api.api_pool)
        .await
        .expect("native role");
    assert_eq!(user, "sagat_app");
    let store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    f.drive().await;
    assert_eq!(
        store
            .certify_leaf_collection_success(&command(&f).await)
            .await,
        Err(LeafCollectionSuccessError::SourceNotClosed)
    );
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0]
            .instance
            .as_ref()
            .expect("scoped task")
            .instance_path[0]
            .scope_node_id,
        "lines"
    );
    f.complete(&tasks[0], json!({"accepted":true})).await;
    f.drive().await;
    let c = command(&f).await;
    let before = selected_sources(&f).await;
    sqlx::raw_sql("CREATE FUNCTION refuse_leaf_native_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='collection.leaf_success_certified' THEN IF NOT EXISTS(SELECT 1 FROM leaf_collection_success_certifications WHERE audit_id=NEW.id) THEN RAISE EXCEPTION 'test expected prior uncommitted certification' USING ERRCODE='P0001'; END IF; RAISE EXCEPTION 'test-only leaf audit refusal' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_leaf_native_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_leaf_native_audit();")
        .execute(&f.api.db.pool).await.expect("audit fault after actual uncommitted fact insert");
    let refused = store
        .certify_leaf_collection_success(&c)
        .await
        .expect_err("confirmed audit failure rolls back fact");
    match refused {
        LeafCollectionSuccessError::Storage(sagat_application::error::StoreError::Internal {
            detail,
        }) => assert!(detail.contains("23514"), "expected injected SQLSTATE only"),
        other => panic!("unexpected failure: {other:?}"),
    }
    let empty:i64=sqlx::query_scalar("SELECT count(*) FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("no orphan fact");
    assert_eq!(empty, 0);
    assert_eq!(selected_sources(&f).await, before);
    sqlx::raw_sql("DROP TRIGGER refuse_leaf_native_audit ON audit_records; DROP FUNCTION refuse_leaf_native_audit();")
        .execute(&f.api.db.pool).await.expect("remove test-only audit fault");
    let created = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("actual committed ordinary source certification");
    assert!(!created.replayed);
    assert_eq!(created.record.identity, c.identity);
    assert_ne!(
        created.record.checkpoint_result_digest,
        created.record.aggregate_result_digest
    );
    assert_eq!(selected_sources(&f).await, before);
    assert_eq!(
        command(&f).await.expected_run_revision,
        c.expected_run_revision,
        "metadata writes no execution journal/head"
    );
    let mut stale = c.clone();
    stale.expected_run_revision -= 1;
    let replay = store
        .certify_leaf_collection_success(&stale)
        .await
        .expect("canonical replay precedes creation CAS");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    let later = f.poll().await;
    assert_eq!(later.len(), 1);
    assert_eq!(
        later[0]
            .instance
            .as_ref()
            .expect("later path")
            .instance_path[0]
            .scope_node_id,
        "later"
    );
    f.fail(&later[0], "DECLINED", false).await;
    f.drive().await;
    assert_eq!(f.run_status().await, "FAILED");
    assert_eq!(
        selected_sources(&f).await,
        before,
        "later scope failure does not reopen certified source"
    );
    let replay = store
        .certify_leaf_collection_success(&c)
        .await
        .expect("original canonical fact survives later failure");
    assert!(replay.replayed);
    assert_eq!(replay.record, created.record);
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='collection.leaf_success_certified')")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("one immutable fact/audit");
    assert_eq!(counts, (1, 1));
    f.verify().await;
}

#[path = "leaf_certification_child.rs"]
mod child_source;

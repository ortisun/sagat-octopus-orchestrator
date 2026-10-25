//! A real effectful later scope plans compensation without changing the certified ordinary leaf.

use super::{
    commit_loss::{install_commit_probe, own_guards_held},
    commit_proxy::CommitResolution,
    late_reports::{observe_first, observe_order},
    refusals::counts,
    *,
};
use crate::support::{
    eventually,
    scope_flow::{RunHold, node},
};
use sagat_domain::execution::ExecutionInstanceId;
use sagat_scheduler::CollectionAdvance;
use std::{future::Future, pin::Pin};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

fn effectful_later(name: &str) -> Value {
    let mut d = document(name);
    d["bodies"]["later_line"] = json!({"entrypoint":"work","nodes":[
        {"id":"work","type":"activity","activity":"line.charge","next":"after",
            "compensateWith":"line.refund","reconcileWith":"line.lookup","highRisk":true},
        {"id":"after","type":"activity","activity":"line.notify","next":"returned"},
        {"id":"returned","type":"return","result":"steps.after.output"}
    ]});
    d
}

async fn gated_commit(f: &Flow, gate: i32) -> i32 {
    eventually("actual native writer COMMIT waits on its unique deferred gate", || async {
        sqlx::query_scalar::<_, i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND query='COMMIT' AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid))")
            .bind(gate).fetch_optional(&f.api.db.pool).await.expect("gated native COMMIT backend")
    }).await
}

async fn blocked<F: Future>(operation: Pin<&mut F>, f: &Flow, leader: i32, fragment: &str) {
    let observed = async {
        eventually("follower waits on the actual committing writer's original Run", || async {
            let yes: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND wait_event_type='Lock' AND position($1 in query)>0 AND $2=ANY(pg_blocking_pids(pid)))")
                .bind(fragment).bind(leader).fetch_one(&f.api.db.pool).await.expect("actual committing-Run blocker");
            yes.then_some(())
        }).await;
    };
    tokio::select! {
        _ = operation => panic!("follower bypassed the committing original Run writer"),
        _ = observed => (),
    }
}

#[tokio::test]
async fn later_effectful_saga_planning_and_certification_preserve_selected_source_in_both_orders() {
    for cancel in [false, true] {
        for certifier_first in [false, true] {
            let name = format!("leaf-saga-{cancel}-{certifier_first}");
            let f = Flow::start(&name, effectful_later(&name), json!([{"v":1}])).await;
            sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
                .execute(&f.api.db.pool)
                .await
                .expect("disposable dormant activation");
            f.drive().await;
            let first = f.poll().await;
            assert_eq!(first.len(), 1);
            assert_eq!(
                first[0]
                    .instance
                    .as_ref()
                    .expect("selected path")
                    .instance_path[0]
                    .scope_node_id,
                "lines"
            );
            f.complete(&first[0], json!({"ordinary":true})).await;
            f.drive().await;
            let later = f.poll().await;
            assert_eq!(later.len(), 1);
            assert_eq!(node(&later[0]), "work");
            assert_eq!(
                later[0]
                    .instance
                    .as_ref()
                    .expect("later path")
                    .instance_path[0]
                    .scope_node_id,
                "later"
            );
            f.complete(&later[0], json!({"charged":true})).await;
            f.drive().await;
            let ready: (String, bool) = sqlx::query_as("SELECT status,lease_token_hash IS NULL AND lease_expires_at IS NULL AND leased_by IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['later']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='after'")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("actual later READY successor after a known effect");
            assert_eq!(ready, ("READY".into(), true));
            let source = selected_sources(&f).await;
            if cancel {
                let (status, _) = f
                    .operate(json!({"action":"CANCEL","reason":"cancel after later effect"}))
                    .await;
                assert_eq!(status, reqwest::StatusCode::ACCEPTED);
            } else {
                let after = f.poll().await;
                assert_eq!(after.len(), 1);
                assert_eq!(node(&after[0]), "after");
                f.fail(&after[0], "LATER_DECLINED", false).await;
            }
            f.until_planning().await;
            assert_eq!(selected_sources(&f).await, source);
            assert_eq!(counts(&f).await, (0, 0));
            f.verify().await;
            let c = command(&f).await;
            let store = PgLeafCollectionSuccessStore::new(
                f.api.api_pool.clone(),
                DefinitionCache::new(Arc::new(JsonIrCodec), 8),
            );
            let key = i64::try_from(
                u64::from_be_bytes(
                    Uuid::new_v4().as_bytes()[..8]
                        .try_into()
                        .expect("eight bytes"),
                ) >> 1,
            )
            .expect("positive key");
            if certifier_first {
                install_commit_probe(&f, key, CommitResolution::Committed).await;
            } else {
                let ddl = format!(
                    "CREATE FUNCTION leaf_saga_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER leaf_saga_commit_gate AFTER INSERT ON scope_sagas DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION leaf_saga_commit_probe();"
                );
                sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
                    .execute(&f.api.db.pool)
                    .await
                    .expect("deferred actual saga planning gate");
            }
            let mut gate = f.api.db.pool.begin().await.expect("test commit gate");
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut *gate)
                .await
                .expect("hold unique test gate");
            let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *gate)
                .await
                .expect("gate pid");
            let owner = ExecutionInstanceId::root(NodeId::parse("later").expect("later owner"));
            let planner = f
                .scheduler
                .advance_collection_from_run(f.tenant, f.run, &owner);
            let certifier = store.certify_leaf_collection_success(&c);
            tokio::pin!(planner, certifier);
            let created = if certifier_first {
                // Queue the preliminary restore BEFORE certification. Certification reaching
                // COMMIT then proves that restore released its Run. A subsequent planner wait
                // behind the certifier must therefore be its actual planning transaction.
                let hold = RunHold::take(&f).await;
                let full_run =
                    "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id,";
                let native_run = "SELECT workflow_version_id,run_version,trace_id FROM runs";
                observe_first(planner.as_mut(), &f.api.db.pool, full_run).await;
                observe_order(
                    planner.as_mut(),
                    certifier.as_mut(),
                    &f.api.db.pool,
                    full_run,
                    native_run,
                )
                .await;
                hold.release().await;
                let pid = tokio::select! {
                    _ = certifier.as_mut() => panic!("certifier bypassed its deferred COMMIT gate"),
                    _ = planner.as_mut() => panic!("actual planner bypassed committing certification"),
                    pid = gated_commit(&f, gate_pid) => pid,
                };
                own_guards_held(&f).await;
                blocked(planner.as_mut(), &f, pid, full_run).await;
                gate.rollback()
                    .await
                    .expect("release actual certification COMMIT");
                let created = timeout(WAIT, certifier.as_mut())
                    .await
                    .expect("certifier deadline")
                    .expect("first certification commits");
                assert_eq!(
                    timeout(WAIT, planner.as_mut())
                        .await
                        .expect("planner deadline")
                        .expect("actual saga planner commits"),
                    CollectionAdvance::Compensated
                );
                created
            } else {
                // The outer scheduler first restores the run and releases that transaction.
                // Observe its *actual planning COMMIT*, not that preliminary read transaction.
                let pid = tokio::select! {
                    _ = planner.as_mut() => panic!("saga planner bypassed its deferred COMMIT gate"),
                    pid = gated_commit(&f, gate_pid) => pid,
                };
                own_guards_held(&f).await;
                blocked(
                    certifier.as_mut(),
                    &f,
                    pid,
                    "SELECT workflow_version_id,run_version,trace_id FROM runs",
                )
                .await;
                gate.rollback()
                    .await
                    .expect("release actual saga planning COMMIT");
                assert_eq!(
                    timeout(WAIT, planner.as_mut())
                        .await
                        .expect("planner deadline")
                        .expect("actual saga planner commits"),
                    CollectionAdvance::Compensated
                );
                let fresh = command(&f).await;
                assert!(fresh.expected_run_revision > c.expected_run_revision);
                assert_eq!(
                    timeout(WAIT, certifier.as_mut())
                        .await
                        .expect("certifier deadline"),
                    Err(LeafCollectionSuccessError::StaleRunRevision {
                        current: fresh.expected_run_revision
                    })
                );
                assert_eq!(counts(&f).await, (0, 0));
                store
                    .certify_leaf_collection_success(&fresh)
                    .await
                    .expect("fresh CAS certifies alongside another owner's saga")
            };
            assert!(!created.replayed);
            let headers: Vec<(String,String,String,Option<String>,bool)> = sqlx::query_as("SELECT owner_node,phase,cause_code,outcome,during_stop FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 ORDER BY owner_node")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_all(&f.api.db.pool).await.expect("actual positive saga header");
            assert_eq!(
                headers,
                vec![(
                    "later".into(),
                    "COMPENSATING".into(),
                    if cancel {
                        "RUN_CANCELLED".into()
                    } else {
                        "LATER_DECLINED".into()
                    },
                    None,
                    true
                )]
            );
            let entries: Vec<(String,i32,String,String)> = sqlx::query_as("SELECT owner_node,iteration_index,node,kind FROM scope_saga_entries WHERE tenant_id=$1 AND run_id=$2 ORDER BY owner_node,iteration_index,node")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_all(&f.api.db.pool).await.expect("actual positive compensation entry");
            assert_eq!(
                entries,
                vec![("later".into(), 0, "work".into(), "COMPENSATE".into())]
            );
            let planned: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='scope.compensation_planned' AND node='later' AND instance_path_nodes='{}' AND instance_path_indices='{}'")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("actual scoped planning event");
            assert_eq!(planned, 1);
            assert_eq!(
                selected_sources(&f).await,
                source,
                "actual planner retained selected source and exact saga absence"
            );
            if certifier_first {
                sqlx::raw_sql("DROP TRIGGER leaf_certification_commit_gate ON leaf_collection_success_certifications; DROP FUNCTION leaf_certification_commit_probe();").execute(&f.api.db.pool).await.expect("remove certification test gate");
            } else {
                sqlx::raw_sql("DROP TRIGGER leaf_saga_commit_gate ON scope_sagas; DROP FUNCTION leaf_saga_commit_probe();").execute(&f.api.db.pool).await.expect("remove saga test gate");
            }
            f.drive().await;
            let compensation = f.poll().await;
            assert_eq!(compensation.len(), 1);
            assert_eq!(node(&compensation[0]), "work~c");
            assert_eq!(compensation[0].task_kind, "COMPENSATION");
            assert_eq!(compensation[0].activity_name, "line.refund");
            assert_eq!(
                compensation[0]
                    .instance
                    .as_ref()
                    .expect("compensation path")
                    .instance_path[0]
                    .scope_node_id,
                "later"
            );
            f.complete(&compensation[0], json!({"refunded":true})).await;
            f.drive().await;
            assert_eq!(
                f.run_status().await,
                if cancel { "CANCELLED" } else { "COMPENSATED" }
            );
            let done: (String,Option<String>) = sqlx::query_as("SELECT phase,outcome FROM scope_sagas WHERE tenant_id=$1 AND run_id=$2 AND owner_node='later' AND owner_path_nodes='{}' AND owner_path_indices='{}'")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("actual saga completion");
            assert_eq!(done, ("DONE".into(), Some("COMPENSATED".into())));
            assert_eq!(selected_sources(&f).await, source);
            let replay = store
                .certify_leaf_collection_success(&c)
                .await
                .expect("original canonical fact survives another owner's compensation");
            assert!(replay.replayed);
            assert_eq!(replay.record, created.record);
            assert_eq!(counts(&f).await, (1, 1));
            f.verify().await;
        }
    }
}

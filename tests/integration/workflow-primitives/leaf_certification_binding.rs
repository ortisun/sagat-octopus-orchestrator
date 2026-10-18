//! An ambient binding writer can expire later leases while the certifier holds its Run.

use super::{
    commit_loss::{install_commit_probe, own_guards_held},
    commit_proxy::CommitResolution,
    late_reports::observe_first,
    refusals::{closed, counts, refusal_state},
    *,
};
use crate::support::{eventually, scope_flow::RunHold};
use tokio::time::timeout;

async fn revoke_later_lease(f: &Flow, ns: NamespaceId, step: Uuid) {
    let before: (String, bool) = sqlx::query_as("SELECT status,lease_expires_at>clock_timestamp() AND leased_by IS NOT NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 AND instance_path_nodes=ARRAY['later']::text[] AND instance_path_indices=ARRAY[0]::integer[] AND node='work'")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).bind(step).fetch_one(&f.api.db.pool).await.expect("exact later lease before replacement");
    assert_eq!(before, ("LEASED".into(), true));
    let response = timeout(
        Duration::from_secs(5),
        f.api
            .as_user(
                reqwest::Method::PUT,
                &format!("/namespaces/{ns}/worker-bindings/scope-worker"),
                "alice",
                "acme",
            )
            .json(&json!({"activities":["line.reserve"]}))
            .send(),
    )
    .await
    .expect("binding writer does not wait for original Run")
    .expect("actual binding replacement");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("binding result");
    assert_eq!(body["revoked"]["sessionsClosed"], 1);
    assert_eq!(body["revoked"]["leasesReleased"], 1);
    let after: (String, bool) = sqlx::query_as("SELECT status,lease_expires_at<=clock_timestamp() AND leased_by IS NOT NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3")
        .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).bind(step).fetch_one(&f.api.db.pool).await.expect("exact ambient lease expiry write");
    assert_eq!(after, ("LEASED".into(), true));
}

#[tokio::test]
async fn ambient_binding_replacement_preserves_source_before_proof_and_during_commit() {
    for certification_proved_first in [false, true] {
        let (f, _, _) = closed("leaf-binding-preservation").await;
        let tasks = f.poll().await;
        assert_eq!(tasks.len(), 1);
        let step = Uuid::parse_str(&tasks[0].step_id).expect("later step UUID");
        assert_eq!(
            tasks[0]
                .instance
                .as_ref()
                .expect("scoped lease")
                .instance_path[0]
                .scope_node_id,
            "later"
        );
        let c = command(&f).await;
        let before = refusal_state(&f).await;
        let pool = f.api.db.runtime_pool_with(1).await;
        sqlx::query("SET application_name='sagat-test-leaf-ambient'")
            .execute(&pool)
            .await
            .expect("isolated certifier connection name");
        let store = PgLeafCollectionSuccessStore::new(
            pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let created = if certification_proved_first {
            let key = i64::try_from(
                u64::from_be_bytes(
                    Uuid::new_v4().as_bytes()[..8]
                        .try_into()
                        .expect("eight bytes"),
                ) >> 1,
            )
            .expect("positive key");
            install_commit_probe(&f, key, CommitResolution::Committed).await;
            let mut gate = f.api.db.pool.begin().await.expect("test COMMIT gate");
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut *gate)
                .await
                .expect("hold actual COMMIT gate");
            let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *gate)
                .await
                .expect("gate pid");
            let native = store.clone();
            let command = c.clone();
            let pending =
                tokio::spawn(async move { native.certify_leaf_collection_success(&command).await });
            eventually("certifier completed source proof and is in actual COMMIT", || async {
                let observed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND application_name='sagat-test-leaf-ambient' AND query='COMMIT' AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)))")
                    .bind(gate_pid).fetch_one(&f.api.db.pool).await.expect("actual native COMMIT wait");
                observed.then_some(())
            }).await;
            own_guards_held(&f).await;
            revoke_later_lease(&f, c.identity.namespace_id(), step).await;
            own_guards_held(&f).await;
            assert_eq!(refusal_state(&f).await, before);
            assert_eq!(
                counts(&f).await,
                (0, 0),
                "canonical metadata remains uncommitted"
            );
            gate.rollback().await.expect("release COMMIT gate");
            let created = timeout(Duration::from_secs(5), pending)
                .await
                .expect("commit deadline")
                .expect("native task")
                .expect("actual first commit");
            sqlx::raw_sql("DROP TRIGGER leaf_certification_commit_gate ON leaf_collection_success_certifications; DROP FUNCTION leaf_certification_commit_probe();")
                .execute(&f.api.db.pool).await.expect("remove test gate");
            created
        } else {
            let hold = RunHold::take(&f).await;
            let native = store.certify_leaf_collection_success(&c);
            tokio::pin!(native);
            observe_first(
                native.as_mut(),
                &f.api.db.pool,
                "SELECT workflow_version_id,run_version,trace_id FROM runs",
            )
            .await;
            revoke_later_lease(&f, c.identity.namespace_id(), step).await;
            assert_eq!(refusal_state(&f).await, before);
            assert_eq!(counts(&f).await, (0, 0));
            hold.release().await;
            timeout(Duration::from_secs(5), native.as_mut())
                .await
                .expect("certifier deadline")
                .expect("certifies after actual replacement")
        };
        assert!(!created.replayed);
        assert_eq!(refusal_state(&f).await, before);
        assert_eq!(counts(&f).await, (1, 1));
        let replay = store
            .certify_leaf_collection_success(&c)
            .await
            .expect("source still replays under current scheduler authority");
        assert!(replay.replayed);
        assert_eq!(replay.record, created.record);
        assert_eq!(counts(&f).await, (1, 1));
        f.verify().await;
        pool.close().await;
    }
}

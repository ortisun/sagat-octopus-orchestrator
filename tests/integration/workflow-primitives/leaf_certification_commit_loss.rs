//! Actual lost COMMIT replies and cold direct reconstruction. No process-restart claim here.

use super::{
    commit_proxy::{CommitAckLossProxy, CommitResolution},
    refusals::{closed, counts, refusal_state},
    *,
};
use crate::support::eventually;
use tokio::time::timeout;

pub(super) async fn install_commit_probe(f: &Flow, key: i64, resolution: CommitResolution) {
    let outcome = match resolution {
        CommitResolution::Committed => "RETURN NEW;",
        // The relay validates this exact synthetic constraint marker on its ErrorResponse.
        CommitResolution::Aborted23514 => {
            "RAISE EXCEPTION USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort',MESSAGE='test-only forced leaf certification abort';"
        }
    };
    // Generated positive integer plus two fixed clauses only, never untrusted DDL.
    let ddl = format!(
        "CREATE FUNCTION leaf_certification_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); {outcome} END $$; CREATE CONSTRAINT TRIGGER leaf_certification_commit_gate AFTER INSERT ON leaf_collection_success_certifications DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION leaf_certification_commit_probe();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("test-only deferred COMMIT probe");
}

pub(super) async fn waiting_commit(f: &Flow, gate: i32) -> i32 {
    // Reuse the already reviewed one-shot wire relay; its test-only application name is retained.
    eventually("actual leaf COMMIT is executing behind the test gate", || async {
        sqlx::query_scalar::<_, i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND application_name='sagat-test-capacity-commit-loss' AND query='COMMIT' AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid))")
            .bind(gate).fetch_optional(&f.api.db.pool).await.expect("actual COMMIT blocker")
    }).await
}

pub(super) async fn own_guards_held(f: &Flow) {
    let c = command(f).await;
    let run = sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR KEY SHARE NOWAIT",
    )
    .bind(f.tenant.as_uuid())
    .bind(c.identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .execute(&f.api.db.pool)
    .await
    .expect_err("original Run survives uncertain client outcome");
    assert_eq!(crate::support::db_error(run).0, "55P03");
    let shard = sqlx::query(
        "SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR NO KEY UPDATE NOWAIT",
    )
    .bind(f.run.as_uuid())
    .execute(&f.api.db.pool)
    .await
    .expect_err("scheduler SHARE survives uncertain client outcome");
    assert_eq!(crate::support::db_error(shard).0, "55P03");
    sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR SHARE NOWAIT")
        .bind(f.run.as_uuid())
        .execute(&f.api.db.pool)
        .await
        .expect("fence remains shareable with another scheduler action");
}

#[tokio::test]
async fn lost_leaf_commit_reconciles_actual_commit_and_abort_with_cold_same_identity_store() {
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        eprintln!("leaf COMMIT scenario: {resolution:?}");
        let (f, original_store, c) = closed("leaf-commit-loss").await;
        drop(original_store);
        let before = refusal_state(&f).await;
        assert_eq!(counts(&f).await, (0, 0));
        let membership: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("actual flat topology");
        assert_eq!(membership, 0, "original Run, not chain-member fencing");
        let key = i64::try_from(
            u64::from_be_bytes(
                Uuid::new_v4().as_bytes()[..8]
                    .try_into()
                    .expect("eight bytes"),
            ) >> 1,
        )
        .expect("positive advisory key");
        install_commit_probe(&f, key, resolution).await;
        let mut gate = f.api.db.pool.begin().await.expect("test-only COMMIT gate");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *gate)
            .await
            .expect("gate held");
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("gate pid");
        let mut proxy = CommitAckLossProxy::start(&f.api.db, resolution).await;
        let user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&proxy.pool)
            .await
            .expect("proxied role");
        assert_eq!(user, "sagat_app");
        proxy.arm();
        let pool = proxy.pool.clone();
        let original_command = c.clone();
        let original = tokio::spawn(async move {
            PgLeafCollectionSuccessStore::new(pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8))
                .certify_leaf_collection_success(&original_command)
                .await
        });
        proxy.wait_sent().await;
        let committing_pid = waiting_commit(&f, gate_pid).await;
        own_guards_held(&f).await;
        assert_eq!(
            counts(&f).await,
            (0, 0),
            "uncommitted metadata is invisible"
        );
        proxy.lose_reply().await;
        let unknown = timeout(Duration::from_secs(5), original)
            .await
            .expect("client unknown deadline")
            .expect("original task")
            .expect_err("dispatched COMMIT reply lost");
        assert_eq!(
            unknown,
            LeafCollectionSuccessError::CommitOutcomeUnknown {
                identity: c.identity.clone()
            }
        );
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f).await;
        let waiter_pool = f.api.db.runtime_pool_with(2).await;
        let pending_pool = waiter_pool.clone();
        let pending_command = c.clone();
        let mut reconciling = tokio::spawn(async move {
            PgLeafCollectionSuccessStore::new(
                pending_pool,
                DefinitionCache::new(Arc::new(JsonIrCodec), 8),
            )
            .certify_leaf_collection_success(&pending_command)
            .await
        });
        eventually("same original identity waits on unresolved Run before canonical lookup", || async {
            sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND position('SELECT workflow_version_id,run_version,trace_id FROM runs' in query)>0 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)))")
                .bind(committing_pid).fetch_one(&f.api.db.pool).await.expect("same-identity original Run wait").then_some(())
        }).await;
        assert!(
            timeout(Duration::from_millis(100), &mut reconciling)
                .await
                .is_err(),
            "a timeout cannot establish original rollback"
        );
        reconciling.abort();
        assert!(
            reconciling
                .await
                .expect_err("reconciler cancelled")
                .is_cancelled()
        );
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f).await;
        assert_eq!(counts(&f).await, (0, 0));
        assert_eq!(refusal_state(&f).await, before);
        gate.rollback().await.expect("release deferred COMMIT");
        assert_eq!(
            proxy.resolution().await,
            resolution,
            "actual backend completion/abort and ReadyForQuery idle"
        );
        proxy.close().await;
        timeout(Duration::from_secs(5), waiter_pool.close())
            .await
            .expect("cancelled waiter pool closes after original resolution");
        let expected = if resolution == CommitResolution::Committed {
            (1, 1)
        } else {
            (0, 0)
        };
        assert_eq!(counts(&f).await, expected);
        let retained_audit: Option<Uuid> = sqlx::query_scalar("SELECT audit_id FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_optional(&f.api.db.pool).await.expect("retained original receipt after actual resolution");
        assert_eq!(
            retained_audit.is_some(),
            resolution == CommitResolution::Committed
        );
        sqlx::raw_sql("DROP TRIGGER leaf_certification_commit_gate ON leaf_collection_success_certifications; DROP FUNCTION leaf_certification_commit_probe();")
            .execute(&f.api.db.pool).await.expect("remove test-only fault");
        // Fresh authority/revision discovery, new pool/cache/store; no in-memory receipt survives.
        let fresh_command = command(&f).await;
        assert_eq!(fresh_command.identity, c.identity);
        let direct_pool = f.api.db.runtime_pool_with(2).await;
        let direct = PgLeafCollectionSuccessStore::new(
            direct_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let recovered = timeout(
            Duration::from_secs(5),
            direct.certify_leaf_collection_success(&fresh_command),
        )
        .await
        .expect("fresh reconstruction deadline")
        .expect("same original identity reconciles actual server outcome");
        assert_eq!(
            recovered.replayed,
            resolution == CommitResolution::Committed
        );
        if let Some(audit) = retained_audit {
            assert_eq!(recovered.record.audit_id.as_uuid(), audit);
        }
        assert_eq!(counts(&f).await, (1, 1));
        assert_eq!(refusal_state(&f).await, before);
        let replay = direct
            .certify_leaf_collection_success(&fresh_command)
            .await
            .expect("canonical record stable after reconstruction");
        assert!(replay.replayed);
        assert_eq!(replay.record, recovered.record);
        assert_eq!(counts(&f).await, (1, 1));
        f.verify().await;
        direct_pool.close().await;
    }
}

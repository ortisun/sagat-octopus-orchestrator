//! Actual unresolved COMMIT with a lost client reply. No production proxy or quota delta.
use super::commit_proxy::{CommitAckLossProxy, CommitResolution};
use super::*;
use crate::support::eventually;
use tokio::time::{Duration, timeout};

async fn peer(n: &NativeFixture) -> (Uuid, RunId) {
    let (_, chain) = spawn_native_child(&n.f.w, false).await;
    let report = n.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    assert_eq!(report.failed, 0);
    assert_eq!(complete_native_activity(n).await, chain);
    let member = Uuid::new_v4();
    let mut tx =
        n.f.w
            .db
            .pool
            .begin()
            .await
            .expect("owner-only two-member baseline");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(member).bind(n.f.w.namespace.id().as_uuid()).bind(chain.as_uuid()).execute(&mut *tx).await.expect("actual second child binding");
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=2 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&mut *tx)
    .await
    .expect("fixture group count");
    tx.commit().await.expect("two actual child members");
    (member, chain)
}

async fn install_commit_probe(f: &Fixture, key: i64, resolution: CommitResolution) {
    let outcome = match resolution {
        CommitResolution::Committed => "RETURN NEW;",
        CommitResolution::Aborted23514 => {
            "RAISE EXCEPTION USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort',MESSAGE='test-only forced capacity Finish commit abort';"
        }
    };
    // Only a generated positive integer and the two fixed clauses above enter this test DDL.
    let ddl = format!(
        "CREATE FUNCTION capacity_finish_proxy_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); {outcome} END $$; CREATE CONSTRAINT TRIGGER capacity_finish_proxy_commit_gate AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION capacity_finish_proxy_commit_probe();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.w.db.pool)
        .await
        .expect("labelled test-only deferred COMMIT probe");
}
async fn waiting_commit(f: &Fixture, gate: i32) -> i32 {
    eventually("server is executing actual COMMIT deferred trigger behind the test gate",||async {
        sqlx::query_scalar::<_,i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND application_name='sagat-test-capacity-commit-loss' AND query='COMMIT' AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid))")
            .bind(gate).fetch_optional(&f.w.db.pool).await.expect("actual COMMIT and blocker observation")
    }).await
}
async fn own_guards_remain_held(f: &Fixture, head: RunId) {
    for statement in [
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE NOWAIT",
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE NOWAIT",
    ] {
        error_is(
            sqlx::query(statement)
                .bind(f.w.admin.tenant_id().as_uuid())
                .bind(f.w.namespace.id().as_uuid())
                .bind(head.as_uuid())
                .execute(&f.w.db.pool)
                .await
                .expect_err("own execution guard retained through actual COMMIT"),
            "55P03",
            None,
        );
    }
    error_is(sqlx::query("SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&f.w.db.pool).await.expect_err("G retained through actual COMMIT"),"55P03",None);
    error_is(
        sqlx::query(
            "SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR UPDATE NOWAIT",
        )
        .bind(head.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect_err("own shard SHARE retained through actual COMMIT"),
        "55P03",
        None,
    );
}

#[tokio::test]
async fn unknown_commit_reconciles_actual_commit_and_abort_without_inventing_a_new_key() {
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let n = super::barriers::runtime_fixture().await;
        let head = complete_native_activity(&n).await;
        let (peer_member, peer_head) = peer(&n).await;
        let command = native_command(&n, head).await;
        let peer_command = native_member_command(&n.f, peer_member, peer_head, peer_head).await;
        let before = execution_snapshot(&n.f).await;
        let key = i64::try_from(
            u64::from_be_bytes(
                Uuid::new_v4().as_bytes()[..8]
                    .try_into()
                    .expect("eight random bytes"),
            ) >> 1,
        )
        .expect("positive advisory key");
        install_commit_probe(&n.f, key, resolution).await;
        let mut gate =
            n.f.w
                .db
                .pool
                .begin()
                .await
                .expect("test-only COMMIT advisory gate");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *gate)
            .await
            .expect("test gate held");
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("gate pid");
        let mut proxy = CommitAckLossProxy::start(&n.f.w.db, resolution).await;
        let user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&proxy.pool)
            .await
            .expect("proxied native role");
        assert_eq!(user, "sagat_app");
        proxy.arm();
        let pool = proxy.pool.clone();
        let pending_command = command.clone();
        let original = tokio::spawn(async move {
            PgCapacityFinishStore::new(pool)
                .finish_current_child_root(&pending_command)
                .await
        });
        proxy.wait_sent().await;
        let committing_pid = waiting_commit(&n.f, gate_pid).await;
        own_guards_remain_held(&n.f, head).await;
        assert_eq!(
            finish_counts(&n.f).await,
            (0, 0, 0),
            "uncommitted writes remain invisible"
        );
        // Close only the reply socket while upstream COMMIT is still executing. SQLx observes
        // EOF, while PostgreSQL still holds the original chain/run/G/shard and unresolved writes.
        proxy.lose_reply().await;
        let unknown = timeout(Duration::from_secs(5), original)
            .await
            .expect("native unknown outcome deadline")
            .expect("original task")
            .expect_err("COMMIT response was lost");
        assert_eq!(
            unknown,
            CapacityFinishError::CommitOutcomeUnknown {
                operation_key: command.operation_key
            }
        );
        assert_eq!(waiting_commit(&n.f, gate_pid).await, committing_pid);

        let direct = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
        assert_eq!(
            timeout(
                Duration::from_secs(2),
                direct.finish_current_child_root(&peer_command)
            )
            .await
            .expect("distinct member reaches G NOWAIT"),
            Err(CapacityFinishError::Busy)
        );
        // Different own-chain/run guards must have been released even though the original G is held.
        sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE NOWAIT")
            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(peer_head.as_uuid()).execute(&n.f.w.db.pool).await.expect("Busy peer releases own chain");
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE NOWAIT")
            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(peer_head.as_uuid()).execute(&n.f.w.db.pool).await.expect("Busy peer releases own run");

        let peer_state:(String,Option<time::OffsetDateTime>)=sqlx::query_as("SELECT state,finished_at FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(peer_member).fetch_one(&n.f.w.db.pool).await.expect("Busy peer membership unchanged");
        assert_eq!(peer_state, ("ACTIVE".into(), None));

        let pool = n.f.w.execution_pool.clone();
        let same = command.clone();
        let mut reconciling = tokio::spawn(async move {
            PgCapacityFinishStore::new(pool)
                .finish_current_child_root(&same)
                .await
        });
        eventually("same-command reconciliation waits on unresolved original own chain, before G",||async {
            sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND position('FROM execution_chains' in query)>0 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)))")
                .bind(committing_pid).fetch_one(&n.f.w.db.pool).await.expect("same-command prefix wait").then_some(())
        }).await;
        assert!(
            timeout(Duration::from_millis(100), &mut reconciling)
                .await
                .is_err(),
            "prefix timeout cannot establish original rollback"
        );
        reconciling.abort();
        assert!(
            reconciling
                .await
                .expect_err("reconciliation was cancelled")
                .is_cancelled()
        );
        assert_eq!(waiting_commit(&n.f, gate_pid).await, committing_pid);
        assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
        own_guards_remain_held(&n.f, head).await;
        gate.rollback()
            .await
            .expect("release test-only COMMIT gate");
        assert_eq!(
            proxy.resolution().await,
            resolution,
            "actual C/E plus ReadyForQuery idle, never delivered to client"
        );
        proxy.close().await;
        match resolution {
            CommitResolution::Committed => {
                assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
                assert_eq!(
                    state(&n.f.w.db.pool, &n.f).await,
                    (3, 2, 1, "VACANT".into(), "FINISHED".into())
                );
            }
            CommitResolution::Aborted23514 => {
                assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
                assert_eq!(
                    state(&n.f.w.db.pool, &n.f).await,
                    (2, 1, 2, "HELD".into(), "ACTIVE".into())
                );
            }
        }
        assert_eq!(execution_snapshot(&n.f).await, before);
        sqlx::raw_sql("DROP TRIGGER capacity_finish_proxy_commit_gate ON capacity_group_finish_receipts; DROP FUNCTION capacity_finish_proxy_commit_probe();")
            .execute(&n.f.w.db.pool).await.expect("remove labelled probe");
        let recovered = direct
            .finish_current_child_root(&command)
            .await
            .expect("same original command after confirmed server resolution");
        assert_eq!(
            recovered.replayed,
            resolution == CommitResolution::Committed
        );
        assert_eq!(
            (
                recovered.revision,
                recovered.epoch,
                recovered.active_members
            ),
            (3, 2, 1)
        );
        assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
        assert_eq!(execution_snapshot(&n.f).await, before);
        crate::hint_contract::assert_outbox_matches_the_contract(&n.f.w.db.pool).await;
    }
}

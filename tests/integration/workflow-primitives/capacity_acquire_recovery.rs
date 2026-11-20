//! Acquire rollback, durable recovery and controlled contention.
use super::*;

#[tokio::test]
async fn busy_group_rolls_back_and_releases_own_chain_and_run_guards() {
    crate::support::bounded::within("busy_group_rolls_back_and_releases_own_chain_and_run_guards", crate::support::bounded::LIFECYCLE_LIMIT, async {
            use tokio::time::{Duration, timeout};
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let mut gate = n.f.w.db.pool.begin().await.expect("group barrier");
            sqlx::query(
                "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
            )
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(n.f.group)
            .execute(&mut *gate)
            .await
            .expect("hold group");
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            assert_eq!(
                timeout(Duration::from_secs(2), store.acquire_current_child_root(&c))
                    .await
                    .expect("NOWAIT"),
                Err(Error::Busy)
            );
            let mut probe = n.f.w.db.pool.begin().await.expect("lock release probe");
            sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain).execute(&mut *probe).await.expect("chain released");
            sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid())
                .bind(n.f.chain)
                .execute(&mut *probe)
                .await
                .expect("run released");
            probe.rollback().await.expect("probe ends");
            gate.rollback().await.expect("release group");
            assert_eq!(counts(&n.f).await, (0, 0, 0));
            store
                .acquire_current_child_root(&c)
                .await
                .expect("same command retries from fresh transaction");
        }).await;
}

#[tokio::test]
async fn audit_outbox_and_receipt_refusals_roll_back_before_same_key_retry() {
    crate::support::bounded::within(
        "Acquire rollback at every write boundary",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            for (install, remove) in [
                (
                    "CREATE TRIGGER refuse_acquire_write BEFORE INSERT ON audit_records FOR EACH ROW WHEN (NEW.action='capacity.member.acquired') EXECUTE FUNCTION refuse_acquire_write()",
                    "DROP TRIGGER refuse_acquire_write ON audit_records",
                ),
                (
                    "CREATE TRIGGER refuse_acquire_write BEFORE INSERT ON event_outbox FOR EACH ROW WHEN (NEW.event_type='capacity.member.acquired') EXECUTE FUNCTION refuse_acquire_write()",
                    "DROP TRIGGER refuse_acquire_write ON event_outbox",
                ),
                (
                    "CREATE TRIGGER refuse_acquire_write BEFORE INSERT ON capacity_group_transfer_receipts FOR EACH ROW WHEN (NEW.kind='ACQUIRE') EXECUTE FUNCTION refuse_acquire_write()",
                    "DROP TRIGGER refuse_acquire_write ON capacity_group_transfer_receipts",
                ),
            ] {
                let n = vacant_fixture().await;
                let c = command(&n).await;
                let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
                let before = execution_snapshot(&n.f).await;
                sqlx::raw_sql("CREATE FUNCTION refuse_acquire_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'controlled Acquire write refusal' USING ERRCODE='23514'; END $$")
                            .execute(&n.f.w.db.pool).await.expect("owned write fault");
                sqlx::query(install)
                    .execute(&n.f.w.db.pool)
                    .await
                    .expect("selected write boundary");
                assert!(matches!(
                    store.acquire_current_child_root(&c).await,
                    Err(Error::Storage(_))
                ));
                assert_eq!(
                    state(&n.f.w.db.pool, &n.f).await,
                    (2, 2, 1, "VACANT".into(), "ACTIVE".into())
                );
                assert_eq!(counts(&n.f).await, (0, 0, 0));
                assert_eq!(execution_snapshot(&n.f).await, before);
                sqlx::query(remove)
                    .execute(&n.f.w.db.pool)
                    .await
                    .expect("remove owned fault");
                store
                    .acquire_current_child_root(&c)
                    .await
                    .expect("same command retries atomically");
                assert_eq!(counts(&n.f).await, (1, 1, 1));
            }
        },
    ).await;
}

#[tokio::test]
async fn lost_commit_ack_reconciles_actual_commit_or_abort_with_the_original_key() {
    crate::support::bounded::within("lost_commit_ack_reconciles_actual_commit_or_abort_with_the_original_key", crate::support::bounded::LIFECYCLE_LIMIT, async {
            use super::super::commit_proxy::{CommitAckLossProxy, CommitResolution};
            use crate::support::eventually;
            use tokio::time::{Duration, timeout};
            for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
                let n = vacant_fixture().await;
                let c = command(&n).await;
                let before = execution_snapshot(&n.f).await;
                sqlx::raw_sql("CREATE TABLE acquire_commit_outcome(abort boolean NOT NULL); GRANT SELECT ON acquire_commit_outcome TO sagat_app; CREATE FUNCTION acquire_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(9407001); IF (SELECT abort FROM acquire_commit_outcome) THEN RAISE EXCEPTION 'controlled Acquire commit abort' USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort'; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER acquire_commit_probe AFTER INSERT ON capacity_group_transfer_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION acquire_commit_probe();")
                    .execute(&n.f.w.db.pool).await.expect("controlled deferred native commit probe");
                sqlx::query("INSERT INTO acquire_commit_outcome VALUES($1)")
                    .bind(resolution == CommitResolution::Aborted23514)
                    .execute(&n.f.w.db.pool)
                    .await
                    .expect("selected real server outcome");
                let mut gate = n.f.w.db.pool.begin().await.expect("commit barrier");
                sqlx::query("SELECT pg_advisory_xact_lock(9407001)")
                    .execute(&mut *gate)
                    .await
                    .expect("hold commit gate");
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *gate)
                    .await
                    .expect("gate pid");
                let mut proxy = CommitAckLossProxy::start(&n.f.w.db, resolution).await;
                proxy.arm();
                let pool = proxy.pool.clone();
                let original = c.clone();
                let pending = tokio::spawn(async move {
                    PgCapacityAcquireStore::new(pool)
                        .acquire_current_child_root(&original)
                        .await
                });
                proxy.wait_sent().await;
                eventually("actual COMMIT is still executing behind the owned gate",||async {
                    let seen:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND application_name='sagat-test-capacity-commit-loss' AND query='COMMIT' AND $1=ANY(pg_blocking_pids(pid)))")
                        .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("actual commit wait");seen.then_some(())
                }).await;
                assert_eq!(counts(&n.f).await, (0, 0, 0));
                proxy.lose_reply().await;
                assert_eq!(
                    timeout(Duration::from_secs(5), pending)
                        .await
                        .expect("unknown outcome deadline")
                        .expect("native task"),
                    Err(Error::CommitOutcomeUnknown {
                        operation_key: c.operation_key
                    })
                );
                gate.rollback()
                    .await
                    .expect("release unresolved server COMMIT");
                assert_eq!(proxy.resolution().await, resolution);
                proxy.close().await;
                let expected = i64::from(resolution == CommitResolution::Committed);
                assert_eq!(counts(&n.f).await, (expected, expected, expected));
                assert_eq!(execution_snapshot(&n.f).await, before);
                sqlx::query("UPDATE acquire_commit_outcome SET abort=false")
                    .execute(&n.f.w.db.pool)
                    .await
                    .expect("clear controlled abort for original-key retry");
                let reconciled = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                    .acquire_current_child_root(&c)
                    .await
                    .expect("same key reconciles server outcome");
                assert_eq!(
                    reconciled.replayed,
                    resolution == CommitResolution::Committed
                );
                assert_eq!(counts(&n.f).await, (1, 1, 1));
            }
        }).await;
}

#[tokio::test]
async fn two_real_members_contend_in_both_orders_then_recheck_cas_and_occupancy() {
    use crate::support::eventually;
    use tokio::time::{Duration, timeout};
    crate::support::bounded::within(
        "Acquire two member contention",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            for peer_first in [false, true] {
                let n = vacant_fixture().await;
                let (_, peer) = spawn_native_child(&n.f.w, false).await;
                let round = n.scheduler.tick().await;
                assert_eq!(round.children_started, 1, "{round:?}");
                assert_eq!(round.failed, 0);
                let peer_member = Uuid::new_v4();
                // Only dormant membership is owner-seeded; both children use real creation.
                let mut seed =
                    n.f.w
                        .db
                        .pool
                        .begin()
                        .await
                        .expect("two active members fixture");
                sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
                            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(peer_member).bind(n.f.w.namespace.id().as_uuid()).bind(peer.as_uuid()).execute(&mut *seed).await.expect("peer membership");
                sqlx::query("UPDATE capacity_groups SET revision=3,active_members=2 WHERE tenant_id=$1 AND group_id=$2")
                            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).execute(&mut *seed).await.expect("fixture count");
                seed.commit().await.expect("peer fixture committed");
                let peer_command = native_member_command(&n.f, peer_member, peer, peer).await;
                let own_command = command(&n).await;
                let peer_command = CapacityAcquireCommand {
                    operation_key: peer_command.operation_key,
                    identity: peer_command.identity,
                    expected: peer_command.expected,
                    scheduler: peer_command.scheduler,
                };
                let (winner, loser) = if peer_first {
                    (peer_command, own_command)
                } else {
                    (own_command, peer_command)
                };
                sqlx::raw_sql("CREATE FUNCTION acquire_receipt_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind='ACQUIRE' THEN PERFORM pg_advisory_xact_lock(9407002); END IF; RETURN NEW; END $$; CREATE TRIGGER acquire_receipt_gate BEFORE INSERT ON capacity_group_transfer_receipts FOR EACH ROW EXECUTE FUNCTION acquire_receipt_gate();")
                            .execute(&n.f.w.db.pool).await.expect("controlled pre-commit gate");
                let mut gate = n.f.w.db.pool.begin().await.expect("acquisition barrier");
                sqlx::query("SELECT pg_advisory_xact_lock(9407002)")
                    .execute(&mut *gate)
                    .await
                    .expect("hold gate");
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *gate)
                    .await
                    .expect("barrier pid");
                let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
                let pending_store = store.clone();
                let original = winner.clone();
                let pending =
                    tokio::spawn(async move { pending_store.acquire_current_child_root(&original).await });
                eventually("winner holds group while receipt waits on controlled gate",||async {
                            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND wait_event_type='Lock' AND position('INSERT INTO capacity_group_transfer_receipts' in query)>0 AND $1=ANY(pg_blocking_pids(pid)))")
                                .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("receipt barrier observed"); waiting.then_some(())
                        }).await;
                let before = execution_snapshot(&n.f).await;
                assert_eq!(
                    timeout(
                        Duration::from_secs(5),
                        store.acquire_current_child_root(&loser)
                    )
                    .await
                    .expect("loser NOWAIT"),
                    Err(Error::Busy)
                );
                assert_eq!(counts(&n.f).await, (0, 0, 0));
                gate.rollback().await.expect("allow first commit");
                let result = pending.await.expect("winner task").expect("winner commits");
                assert_eq!(
                    (result.revision, result.epoch, result.active_members),
                    (4, 3, 2)
                );
                assert_eq!(
                    result.possession,
                    CapacityPossession::Held(winner.identity.member_id)
                );
                assert_eq!(
                    store.acquire_current_child_root(&loser).await,
                    Err(Error::StaleGroupRevision { current: 4 })
                );
                let mut fresh = loser.clone();
                fresh.expected.group_revision = 4;
                fresh.expected.group_epoch = 3;
                assert_eq!(
                    store.acquire_current_child_root(&fresh).await,
                    Err(Error::Occupied)
                );
                assert!(
                    store
                        .acquire_current_child_root(&winner)
                        .await
                        .expect("winner receipt")
                        .replayed
                );
                assert_eq!(counts(&n.f).await, (1, 1, 1));
                assert_eq!(execution_snapshot(&n.f).await, before);
                assert_eq!(
                    header(&n.f).await,
                    (
                        4,
                        3,
                        2,
                        "HELD".into(),
                        Some(winner.identity.member_id.as_uuid())
                    )
                );
                let active: Vec<(Uuid,String)> = sqlx::query_as("SELECT member_id,state FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 ORDER BY member_id")
                            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).fetch_all(&n.f.w.db.pool).await.expect("both active memberships");
                let mut expected = vec![
                    (n.f.member, "ACTIVE".to_owned()),
                    (peer_member, "ACTIVE".to_owned()),
                ];
                expected.sort();
                assert_eq!(active, expected);
            }
        },
    ).await;
}

#[tokio::test]
async fn scheduler_expiry_while_chain_is_held_refuses_application_and_replay() {
    crate::support::bounded::within("Acquire fence after chain wait", crate::support::bounded::LIFECYCLE_LIMIT, async {
            use crate::support::eventually;
            for replay in [false, true] {
                let n = vacant_fixture().await;
                let c = command(&n).await;
                if replay {
                    PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                        .acquire_current_child_root(&c)
                        .await
                        .expect("original acquire");
                }
                let mut gate = n.f.w.db.pool.begin().await.expect("chain expiry barrier");
                sqlx::query(
                    "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
                )
                .bind(n.f.w.admin.tenant_id().as_uuid())
                .bind(n.f.chain)
                .execute(&mut *gate)
                .await
                .expect("hold chain");
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *gate)
                    .await
                    .expect("gate pid");
                let pool = n.f.w.execution_pool.clone();
                let c = c.clone();
                let task = tokio::spawn(async move {
                    PgCapacityAcquireStore::new(pool)
                        .acquire_current_child_root(&c)
                        .await
                });
                eventually("native command waits before sampling current scheduler time",||async {
                    let seen:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) AND position('FROM execution_chains' in query)>0)")
                        .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("actual chain wait");seen.then_some(())
                }).await;
                sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second' WHERE shard=run_shard($1)").bind(n.f.chain).execute(&n.f.w.db.pool).await.expect("expire unchanged scheduler identity");
                gate.rollback().await.expect("release chain after expiry");
                assert_eq!(
                    task.await.expect("native task"),
                    Err(Error::SchedulerFenceLost)
                );
                let expected = i64::from(replay);
                assert_eq!(counts(&n.f).await, (expected, expected, expected));
            }
        }).await;
}

#[tokio::test]
async fn a_real_deferred_foreign_key_abort_is_confirmed_and_rolls_back_every_write() {
    crate::support::bounded::within(
        "Acquire confirmed deferred foreign key abort",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let before = execution_snapshot(&n.f).await;
            // Test-only instrumentation causes a real PostgreSQL FK violation at COMMIT,
            // rather than simulating a database error or losing the acknowledgement.
            sqlx::raw_sql("CREATE TABLE acquire_fk_parent(id uuid PRIMARY KEY); CREATE TABLE acquire_fk_probe(id uuid REFERENCES acquire_fk_parent(id) DEFERRABLE INITIALLY DEFERRED); GRANT SELECT ON acquire_fk_parent TO sagat_app; GRANT INSERT ON acquire_fk_probe TO sagat_app; CREATE FUNCTION acquire_fk_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN INSERT INTO acquire_fk_probe VALUES(NEW.operation_key); RETURN NEW; END $$; CREATE TRIGGER acquire_fk_probe AFTER INSERT ON capacity_group_transfer_receipts FOR EACH ROW WHEN (NEW.kind='ACQUIRE') EXECUTE FUNCTION acquire_fk_probe();")
                .execute(&n.f.w.db.pool).await.expect("real deferred FK probe");
            assert_eq!(store.acquire_current_child_root(&c).await, Err(Error::Storage(sagat_application::error::StoreError::MissingReference)));
            assert_eq!(header(&n.f).await, (2, 2, 1, "VACANT".into(), None));
            assert_eq!(counts(&n.f).await, (0, 0, 0));
            assert_eq!(execution_snapshot(&n.f).await, before);
            let probe: i64 = sqlx::query_scalar("SELECT count(*) FROM acquire_fk_probe")
                .fetch_one(&n.f.w.db.pool).await.expect("aborted FK probe");
            assert_eq!(probe, 0);
            sqlx::query("INSERT INTO acquire_fk_parent VALUES($1)").bind(c.operation_key)
                .execute(&n.f.w.db.pool).await.expect("satisfy owned test constraint");
            let result = store.acquire_current_child_root(&c).await.expect("same command after confirmed abort");
            assert!(!result.replayed);
            assert_eq!(header(&n.f).await, (3, 3, 1, "HELD".into(), Some(n.f.member)));
            assert_eq!(counts(&n.f).await, (1, 1, 1));
            assert_eq!(execution_snapshot(&n.f).await, before);
        },
    ).await;
}

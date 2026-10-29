//! Native dormant Vacate: actual child creation, runtime role, durable receipts.
use super::*;
use sagat_application::capacity_vacate::{
    CapacityVacateCommand, CapacityVacateError as Error, CurrentChildRootVacateStore,
};
use sagat_infra_postgres::PgCapacityVacateStore;

async fn command(n: &NativeFixture) -> CapacityVacateCommand {
    let c = native_command(n, RunId::new(n.f.chain)).await;
    CapacityVacateCommand {
        operation_key: c.operation_key,
        identity: c.identity,
        expected: c.expected,
        scheduler: c.scheduler,
    }
}
pub(super) async fn untouched(f: &Fixture) -> Vec<(String, String, String)> {
    sqlx::query_as("SELECT 'runs',id::text,xmin::text FROM runs WHERE tenant_id=$1 UNION ALL SELECT 'steps',md5(to_jsonb(s)::text),xmin::text FROM steps s WHERE tenant_id=$1 UNION ALL SELECT 'chains',chain_id::text,xmin::text FROM execution_chains WHERE tenant_id=$1 UNION ALL SELECT 'members',member_id::text,xmin::text FROM capacity_group_members WHERE tenant_id=$1 UNION ALL SELECT 'tenant_usage',tenant_id::text,xmin::text FROM tenant_iteration_usage WHERE tenant_id=$1 UNION ALL SELECT 'retained_usage',tenant_id::text,xmin::text FROM tenant_retained_usage WHERE tenant_id=$1 ORDER BY 1,2")
        .bind(f.w.admin.tenant_id().as_uuid()).fetch_all(&f.w.db.pool).await.expect("untouched row versions")
}
async fn counts(f: &Fixture) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$3 AND action='capacity.member.vacated'),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='capacity.member.vacated')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(f.group.to_string()).fetch_one(&f.w.db.pool).await.expect("vacate side effects")
}
#[tokio::test]
async fn a_ready_child_vacates_atomically_without_rewriting_execution_or_membership() {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let before = execution_snapshot(&n.f).await;
    let versions = untouched(&n.f).await;
    let result = PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&c)
        .await
        .expect("ready child is quiescent");
    assert_eq!(
        (
            result.revision,
            result.epoch,
            result.active_members,
            result.possession,
            result.replayed
        ),
        (2, 2, 1, CapacityPossession::Vacant, false)
    );
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (2, 2, 1, "VACANT".into(), "ACTIVE".into())
    );
    assert_eq!(counts(&n.f).await, (1, 1, 1));
    let hints =
        super::super::hint_contract::assert_outbox_matches_the_contract(&n.f.w.db.pool).await;
    super::super::hint_contract::assert_published(&hints, &["capacity.member.vacated"]);
    assert_eq!(execution_snapshot(&n.f).await, before);
    assert_eq!(untouched(&n.f).await, versions);
}
#[tokio::test]
async fn exact_replay_returns_the_original_transition_once() {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    let a = store.vacate_current_child_root(&c).await.expect("vacate");
    let mut b = store
        .vacate_current_child_root(&c)
        .await
        .expect("receipt before gate");
    assert!(b.replayed);
    b.replayed = false;
    assert_eq!(a, b);
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}
#[tokio::test]
async fn final_child_is_refused_without_replacing_finish() {
    let n = super::barriers::runtime_fixture().await;
    complete_native_activity(&n).await;
    let c = command(&n).await;
    let before = execution_snapshot(&n.f).await;
    assert_eq!(
        PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
            .vacate_current_child_root(&c)
            .await,
        Err(Error::ChainFinal)
    );
    assert_eq!(execution_snapshot(&n.f).await, before);
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
}

#[tokio::test]
async fn actual_worker_lease_including_an_expired_one_blocks_vacate() {
    let n = super::barriers::runtime_fixture().await;
    let session = n.f.w.session(&["capacity.work"]).await;
    let tasks = n.f.w.poll(&session, 1).await;
    assert_eq!(tasks.len(), 1);
    for expired in [false, true] {
        if expired {
            n.f.w.age_lease(RunId::new(n.f.chain), "work").await;
        }
        let c = command(&n).await;
        assert_eq!(
            PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
                .vacate_current_child_root(&c)
                .await,
            Err(Error::NotQuiescent)
        );
        assert_eq!(
            state(&n.f.w.db.pool, &n.f).await,
            (1, 1, 1, "HELD".into(), "ACTIVE".into())
        );
    }
    assert_eq!(counts(&n.f).await, (0, 0, 0));
}

#[tokio::test]
async fn mismatched_binding_stale_counters_and_lost_scheduler_are_inert() {
    let n = super::barriers::runtime_fixture().await;
    let original = command(&n).await;
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    let before = execution_snapshot(&n.f).await;
    for field in 0..7 {
        let mut c = original.clone();
        let expected = match field {
            0 => {
                c.expected.group_revision += 1;
                Error::StaleGroupRevision {
                    current: original.expected.group_revision,
                }
            }
            1 => {
                c.expected.group_epoch += 1;
                Error::StaleGroupEpoch {
                    current: original.expected.group_epoch,
                }
            }
            2 => {
                c.expected.chain_revision += 1;
                Error::StaleChainRevision {
                    current: original.expected.chain_revision,
                }
            }
            3 => {
                c.expected.run_revision += 1;
                Error::StaleRunRevision {
                    current: original.expected.run_revision,
                }
            }
            4 => {
                c.expected.run_revision = u64::MAX;
                Error::CounterExhausted
            }
            5 => {
                c.scheduler =
                    SchedulerAuthority::for_run("not-the-owner", c.identity.head_run_id, 1);
                Error::SchedulerFenceLost
            }
            6 => {
                c.identity.scope =
                    ChainScope::new(n.f.w.other_admin.tenant_id(), n.f.w.other_namespace.id());
                Error::Missing
            }
            _ => unreachable!(),
        };
        assert_eq!(
            store.vacate_current_child_root(&c).await,
            Err(expected),
            "refusal {field}"
        );
    }
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    assert_eq!(execution_snapshot(&n.f).await, before);
}

#[tokio::test]
async fn another_namespace_and_a_root_without_child_lineage_are_refused() {
    let n = super::barriers::runtime_fixture().await;
    let mut c = command(&n).await;
    c.identity.scope = ChainScope::new(n.f.w.admin.tenant_id(), n.f.w.other_namespace.id());
    assert_eq!(
        PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
            .vacate_current_child_root(&c)
            .await,
        Err(Error::BindingMismatch)
    );
    let f = fixture().await;
    let _scheduler = super::super::chains::scheduler(&f.w, "root-vacate-refusal").await;
    let old = native_member_command(&f, f.member, RunId::new(f.chain), RunId::new(f.chain)).await;
    let c = CapacityVacateCommand {
        operation_key: old.operation_key,
        identity: old.identity,
        expected: old.expected,
        scheduler: old.scheduler,
    };
    assert_eq!(
        PgCapacityVacateStore::new(f.w.db.runtime_pool().await)
            .vacate_current_child_root(&c)
            .await,
        Err(Error::BindingMismatch)
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    assert_eq!(counts(&f).await, (0, 0, 0));
}

#[tokio::test]
async fn a_new_command_after_vacating_is_not_a_holder_but_the_old_receipt_replays() {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    store
        .vacate_current_child_root(&c)
        .await
        .expect("initial vacate");
    let next = command(&n).await;
    assert_eq!(
        store.vacate_current_child_root(&next).await,
        Err(Error::NotHolder)
    );
    let mut conflicting = c.clone();
    conflicting.expected.run_revision += 1;
    assert_eq!(
        store.vacate_current_child_root(&conflicting).await,
        Err(Error::FingerprintConflict)
    );
    assert!(
        store
            .vacate_current_child_root(&c)
            .await
            .expect("original receipt")
            .replayed
    );
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn original_receipt_survives_actual_child_continuation_and_current_lease() {
    let mut n = native_fixture(true).await;
    n.f.w.execution_pool = n.f.w.db.runtime_pool_with(16).await;
    let c = command(&n).await;
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    let initial = store
        .vacate_current_child_root(&c)
        .await
        .expect("vacate before continuation");
    // Possession is still dormant: this is a real legacy worker/continuation path, not Acquire.
    complete_native_activity(&n).await;
    for _ in 0..8 {
        let round = n.scheduler.tick().await;
        assert_eq!(round.failed, 0);
        if !round.did_work() {
            break;
        }
    }
    let head: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.chain)
    .fetch_one(&n.f.w.db.pool)
    .await
    .expect("new physical head");
    assert_ne!(head, n.f.chain);
    let mut replay = store
        .vacate_current_child_root(&c)
        .await
        .expect("receipt before current head/finality/lease gates");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(replay, initial);
    let mut stale = c.clone();
    stale.operation_key = Uuid::new_v4();
    assert_eq!(
        store.vacate_current_child_root(&stale).await,
        Err(Error::StaleHead {
            current: RunId::new(head)
        })
    );
    let session = n.f.w.session(&["capacity.work"]).await;
    let tasks = n.f.w.poll(&session, 1).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].run_id.as_uuid(), head);
    assert!(
        store
            .vacate_current_child_root(&c)
            .await
            .expect("receipt even with successor leased")
            .replayed
    );
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn busy_group_rolls_back_and_releases_own_chain_and_run_guards() {
    use tokio::time::{Duration, timeout};
    let n = super::barriers::runtime_fixture().await;
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
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        timeout(Duration::from_secs(2), store.vacate_current_child_root(&c))
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
        .vacate_current_child_root(&c)
        .await
        .expect("same command retries from fresh transaction");
}

#[tokio::test]
async fn outbox_refusal_rolls_back_header_audit_and_receipt_before_same_key_retry() {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
    let before = execution_snapshot(&n.f).await;
    sqlx::raw_sql("CREATE FUNCTION refuse_vacate_hint() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type='capacity.member.vacated' THEN RAISE EXCEPTION 'controlled outbox refusal' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_vacate_hint BEFORE INSERT ON event_outbox FOR EACH ROW EXECUTE FUNCTION refuse_vacate_hint();")
        .execute(&n.f.w.db.pool).await.expect("controlled outbox fault");
    assert!(matches!(
        store.vacate_current_child_root(&c).await,
        Err(Error::Storage(_))
    ));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    assert_eq!(execution_snapshot(&n.f).await, before);
    sqlx::query("DROP TRIGGER refuse_vacate_hint ON event_outbox")
        .execute(&n.f.w.db.pool)
        .await
        .expect("remove owned test fault");
    store
        .vacate_current_child_root(&c)
        .await
        .expect("original command retries atomically");
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn concurrent_exact_commands_wait_on_chain_before_run_and_settle_once() {
    use crate::support::eventually;
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let mut gate = n.f.w.db.pool.begin().await.expect("chain gate");
    sqlx::query(
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.chain)
    .execute(&mut *gate)
    .await
    .expect("hold own chain");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate backend");
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let pool = n.f.w.execution_pool.clone();
        let c = c.clone();
        jobs.push(tokio::spawn(async move {
            PgCapacityVacateStore::new(pool)
                .vacate_current_child_root(&c)
                .await
        }));
    }
    eventually("both native commands block on their own chain",|| async {
        let count:i64=sqlx::query_scalar("WITH RECURSIVE blocked(pid) AS (SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) UNION SELECT a.pid FROM pg_stat_activity a JOIN blocked b ON b.pid=ANY(pg_blocking_pids(a.pid)) WHERE a.datname=current_database()) SELECT count(*) FROM pg_stat_activity WHERE pid IN (SELECT pid FROM blocked) AND position('FROM execution_chains' in query)>0")
            .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("actual wait observation");(count==2).then_some(())
    }).await;
    let mut probe = n.f.w.db.pool.begin().await.expect("run probe");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
        .bind(n.f.w.admin.tenant_id().as_uuid())
        .bind(n.f.chain)
        .execute(&mut *probe)
        .await
        .expect("no run lock before chain");
    probe.rollback().await.expect("end probe");
    gate.rollback().await.expect("release chain");
    let mut a = jobs.remove(0).await.expect("task a").expect("result a");
    let mut b = jobs.remove(0).await.expect("task b").expect("result b");
    assert_ne!(a.replayed, b.replayed);
    a.replayed = false;
    b.replayed = false;
    assert_eq!(a, b);
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn a_real_claim_commits_before_vacate_rechecks_revision_and_quiescence() {
    use crate::support::eventually;
    let n = std::sync::Arc::new(super::barriers::runtime_fixture().await);
    let c = command(&n).await;
    sqlx::raw_sql("CREATE TABLE vacate_claim_gate(id integer PRIMARY KEY); INSERT INTO vacate_claim_gate VALUES(1); GRANT SELECT,UPDATE ON vacate_claim_gate TO sagat_app; CREATE FUNCTION gate_vacate_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.status IN ('LEASED','RUNNING') AND OLD.status IS DISTINCT FROM NEW.status THEN PERFORM id FROM vacate_claim_gate WHERE id=1 FOR UPDATE; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER gate_vacate_claim AFTER UPDATE ON steps DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION gate_vacate_claim();")
        .execute(&n.f.w.db.pool).await.expect("deferred actual claim commit barrier");
    let mut gate = n.f.w.db.pool.begin().await.expect("gate transaction");
    sqlx::query("SELECT id FROM vacate_claim_gate WHERE id=1 FOR UPDATE")
        .execute(&mut *gate)
        .await
        .expect("hold commit gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate pid");
    let worker = n.clone();
    let claim = tokio::spawn(async move {
        let session = worker.f.w.session(&["capacity.work"]).await;
        worker.f.w.poll(&session, 1).await
    });
    let claim_pid=eventually("actual worker claim reaches deferred COMMIT",|| async {
        sqlx::query_scalar::<_,i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) AND query='COMMIT' LIMIT 1")
            .bind(gate_pid).fetch_optional(&n.f.w.db.pool).await.expect("blocked claim backend")
    }).await;
    let pool = n.f.w.execution_pool.clone();
    let old = c.clone();
    let vacate = tokio::spawn(async move {
        PgCapacityVacateStore::new(pool)
            .vacate_current_child_root(&old)
            .await
    });
    eventually("Vacate waits behind the real claimant chain guard",|| async {
        let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) AND position('FROM execution_chains' in query)>0)")
            .bind(claim_pid).fetch_one(&n.f.w.db.pool).await.expect("Vacate chain wait");blocked.then_some(())
    }).await;
    gate.rollback().await.expect("claim can commit");
    assert_eq!(claim.await.expect("claim task").len(), 1);
    let current: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(n.f.chain)
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("committed claim revision");
    assert!(u64::try_from(current).expect("revision") > c.expected.run_revision);
    assert_eq!(
        vacate.await.expect("vacate task"),
        Err(Error::StaleRunRevision {
            current: u64::try_from(current).expect("revision")
        })
    );
    let fresh = command(&n).await;
    assert_eq!(
        PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
            .vacate_current_child_root(&fresh)
            .await,
        Err(Error::NotQuiescent)
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
}

#[tokio::test]
async fn scheduler_expiry_while_chain_is_held_refuses_application_and_replay() {
    use crate::support::eventually;
    for replay in [false, true] {
        let n = super::barriers::runtime_fixture().await;
        let c = command(&n).await;
        if replay {
            PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
                .vacate_current_child_root(&c)
                .await
                .expect("original vacate");
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
            PgCapacityVacateStore::new(pool)
                .vacate_current_child_root(&c)
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
}

#[tokio::test]
async fn lost_commit_ack_reconciles_actual_commit_or_abort_with_the_original_key() {
    use super::commit_proxy::{CommitAckLossProxy, CommitResolution};
    use crate::support::eventually;
    use tokio::time::{Duration, timeout};
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let n = super::barriers::runtime_fixture().await;
        let c = command(&n).await;
        let before = execution_snapshot(&n.f).await;
        sqlx::raw_sql("CREATE TABLE vacate_commit_outcome(abort boolean NOT NULL); GRANT SELECT ON vacate_commit_outcome TO sagat_app; CREATE FUNCTION vacate_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(9407001); IF (SELECT abort FROM vacate_commit_outcome) THEN RAISE EXCEPTION 'controlled Vacate commit abort' USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort'; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER vacate_commit_probe AFTER INSERT ON capacity_group_transfer_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION vacate_commit_probe();")
            .execute(&n.f.w.db.pool).await.expect("controlled deferred native commit probe");
        sqlx::query("INSERT INTO vacate_commit_outcome VALUES($1)")
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
            PgCapacityVacateStore::new(pool)
                .vacate_current_child_root(&original)
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
        sqlx::query("UPDATE vacate_commit_outcome SET abort=false")
            .execute(&n.f.w.db.pool)
            .await
            .expect("clear controlled abort for original-key retry");
        let reconciled = PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
            .vacate_current_child_root(&c)
            .await
            .expect("same key reconciles server outcome");
        assert_eq!(
            reconciled.replayed,
            resolution == CommitResolution::Committed
        );
        assert_eq!(counts(&n.f).await, (1, 1, 1));
    }
}

#[tokio::test]
async fn empty_migration_reverses_but_retained_receipts_preserve_the_index_and_evidence() {
    let w = world().await;
    MIGRATOR
        .undo(&w.db.pool, 20261225000000)
        .await
        .expect("empty transfer schema reverses");
    let absent:bool=sqlx::query_scalar("SELECT to_regclass('capacity_group_transfer_receipts') IS NULL AND to_regclass('steps_run_capacity_lease_idx') IS NULL").fetch_one(&w.db.pool).await.expect("empty rollback removes only new objects");
    assert!(absent);
    migrate(&w.db.pool)
        .await
        .expect("reapply empty transfer schema");
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&c)
        .await
        .expect("retained transition");
    let error = MIGRATOR
        .undo(&n.f.w.db.pool, 20261225000000)
        .await
        .expect_err("retained transfer evidence must survive");
    match error {
        sqlx::migrate::MigrateError::ExecuteMigration(error, version) => {
            assert_eq!(version, 20261226000000);
            error_is(error, "55000", None);
        }
        other => panic!("unexpected downgrade error: {other:?}"),
    }
    let index: bool =
        sqlx::query_scalar("SELECT to_regclass('steps_run_capacity_lease_idx') IS NOT NULL")
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("index drop rolls back with retained-data refusal");
    assert!(index);
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn transfer_receipts_are_append_only_and_the_lease_probe_has_an_index_access_path() {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&c)
        .await
        .expect("receipt");
    let permissions:(bool,bool,bool,bool)=sqlx::query_as("SELECT has_table_privilege('sagat_app','capacity_group_transfer_receipts','SELECT'),has_table_privilege('sagat_app','capacity_group_transfer_receipts','INSERT'),has_table_privilege('sagat_app','capacity_group_transfer_receipts','UPDATE'),has_table_privilege('sagat_app','capacity_group_transfer_receipts','DELETE')").fetch_one(&n.f.w.db.pool).await.expect("minimal table privileges");
    assert_eq!(permissions, (true, false, false, false));
    let error = sqlx::query(
        "DELETE FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&n.f.w.execution_pool)
    .await
    .expect_err("runtime cannot delete evidence");
    error_is(error, "42501", None);
    assert!(
        sqlx::query(
            "DELETE FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2"
        )
        .bind(n.f.w.admin.tenant_id().as_uuid())
        .bind(n.f.group)
        .execute(&n.f.w.db.pool)
        .await
        .is_err(),
        "ALWAYS append-only trigger also guards owner"
    );
    let mut tx =
        n.f.w
            .db
            .pool
            .begin()
            .await
            .expect("index availability control");
    // A tiny fixture may naturally prefer a sequential scan. This proves availability, not scale.
    sqlx::query("SET LOCAL enable_seqscan=off")
        .execute(&mut *tx)
        .await
        .expect("local plan control");
    let plan:serde_json::Value=sqlx::query_scalar("EXPLAIN(FORMAT JSON) SELECT EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status IN ('LEASED','RUNNING'))").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain).fetch_one(&mut *tx).await.expect("EXPLAIN actual lease probe");
    assert!(plan.to_string().contains("steps_run_capacity_lease_idx"));
    tx.rollback().await.expect("discard local planner control");
    assert_eq!(counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn a_scoped_path_in_a_mixed_group_cannot_use_child_root_vacate() {
    let n = super::barriers::runtime_fixture().await;
    let member = Uuid::new_v4();
    // Owner-only referential fixture, not scoped admission or a source certification.
    let mut tx = n.f.w.db.pool.begin().await.expect("mixed-kind fixture");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state,original_run_id,root_owner_node,iteration_index) VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,'ACTIVE',$5,'kid',0)")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(member).bind(n.f.w.namespace.id().as_uuid()).bind(n.parent.as_uuid()).execute(&mut *tx).await.expect("valid scoped registry shape");
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=2 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&mut *tx)
    .await
    .expect("mixed group header");
    tx.commit().await.expect("mixed fixture commits");
    let mut c = command(&n).await;
    c.identity.member_id = CapacityMemberId::new(member);
    assert_eq!(
        PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
            .vacate_current_child_root(&c)
            .await,
        Err(Error::BindingMismatch)
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (2, 1, 2, "HELD".into(), "ACTIVE".into())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn vacate_exports_closed_outcomes_and_counts_only_acknowledged_first_transitions() {
    use crate::support::collector::Collector;
    use sagat_telemetry::{Metrics, TraceId};
    use std::time::Duration;
    use tracing::Instrument;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let subscriber =
        tracing_subscriber::registry().with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0));
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    async {
        let parent = tracing::info_span!(target:"sagat_test","vacate.caller",trace_id=%trace);
        async {
            let store = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
            store.vacate_current_child_root(&c).await.expect("applied");
            assert!(
                store
                    .vacate_current_child_root(&c)
                    .await
                    .expect("replayed")
                    .replayed
            );
            let next = command(&n).await;
            assert_eq!(
                store.vacate_current_child_root(&next).await,
                Err(Error::NotHolder)
            );
        }
        .instrument(parent)
        .await;
    }
    .with_subscriber(subscriber)
    .await;
    sink.shutdown(Duration::from_secs(5)).await;
    let spans = collector.named("workflow.capacity.vacate");
    assert_eq!(spans.len(), 3);
    let outcomes: std::collections::BTreeSet<_> = spans
        .iter()
        .map(|s| {
            assert_eq!(s.trace_id, trace.to_string());
            s.attr("sagat.outcome").expect("closed outcome").to_owned()
        })
        .collect();
    assert_eq!(
        outcomes,
        std::collections::BTreeSet::from([
            "applied".to_owned(),
            "replayed".to_owned(),
            "not_holder".to_owned()
        ])
    );
    let page = metrics.render();
    for outcome in ["applied", "replayed", "not_holder"] {
        let total: u64 = page
            .lines()
            .filter(|l| {
                l.starts_with("workflow_capacity_transfer_attempts_total{")
                    && l.contains(&format!("outcome=\"{outcome}\""))
                    && l.contains("operation=\"vacate\"")
            })
            .map(|l| {
                l.rsplit(' ')
                    .next()
                    .expect("sample")
                    .parse::<u64>()
                    .expect("counter")
            })
            .sum();
        assert_eq!(total, 1);
    }
    let transition: u64 = page
        .lines()
        .filter(|l| l.starts_with("workflow_capacity_member_vacated_total "))
        .map(|l| {
            l.rsplit(' ')
                .next()
                .expect("sample")
                .parse::<u64>()
                .expect("counter")
        })
        .sum();
    assert_eq!(transition, 1);
    for line in page.lines().filter(|l| l.starts_with("workflow_capacity_")) {
        assert!(!line.contains(&n.f.group.to_string()));
        assert!(!line.contains(&n.f.w.admin.tenant_id().to_string()));
    }
}

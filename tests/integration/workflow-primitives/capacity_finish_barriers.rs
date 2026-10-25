//! Native transaction barriers, not quota or production lifecycle tests.
use super::*;
use crate::support::eventually;
use tokio::time::{Duration, timeout};

// The adapter under test always uses the runtime role, independently of the wider suite mode.
pub(super) async fn runtime_fixture() -> NativeFixture {
    let mut fixture = native_fixture(false).await;
    fixture.f.w.execution_pool = fixture.f.w.db.runtime_pool_with(16).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&fixture.f.w.execution_pool)
        .await
        .expect("explicit native runtime role");
    assert_eq!(role, "sagat_app");
    fixture
}

async fn lock_chain(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, f: &Fixture) -> i32 {
    sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.w.namespace.id().as_uuid()).bind(f.chain).execute(&mut **tx).await.expect("external own-chain guard");
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **tx)
        .await
        .expect("gate pid")
}

async fn waiting(f: &Fixture, pid: i32, query_fragment: &'static str, count: i64) {
    eventually("native statement actually waits on the controlled tuple",||async {
        let seen:i64=sqlx::query_scalar("WITH RECURSIVE blocked(pid) AS (SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) UNION SELECT a.pid FROM pg_stat_activity a JOIN blocked b ON b.pid=ANY(pg_blocking_pids(a.pid)) WHERE a.datname=current_database()) SELECT count(*) FROM pg_stat_activity WHERE pid IN (SELECT pid FROM blocked) AND usename='sagat_app' AND wait_event_type='Lock' AND position($2 in query)>0")
            .bind(pid).bind(query_fragment).fetch_one(&f.w.db.pool).await.expect("actual blocker observation");
        (seen==count).then_some(())
    }).await;
}

#[tokio::test]
async fn group_busy_consumes_transaction_and_releases_own_chain_and_run() {
    let n = runtime_fixture().await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let mut gate = n.f.w.db.pool.begin().await.expect("external G holder");
    sqlx::query(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&mut *gate)
    .await
    .expect("G held unchanged");
    let before = execution_snapshot(&n.f).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        timeout(
            Duration::from_secs(2),
            store.finish_current_child_root(&command)
        )
        .await
        .expect("G is NOWAIT"),
        Err(CapacityFinishError::Busy)
    );
    let mut probe =
        n.f.w
            .db
            .pool
            .begin()
            .await
            .expect("fresh lock probe while G remains held");
    sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE NOWAIT")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).execute(&mut *probe).await.expect("failed attempt released chain");
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE NOWAIT",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.w.namespace.id().as_uuid())
    .bind(head.as_uuid())
    .execute(&mut *probe)
    .await
    .expect("failed attempt released run");
    probe.rollback().await.expect("probe ends");
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    assert_eq!(execution_snapshot(&n.f).await, before);
    gate.rollback().await.expect("release G");
    store
        .finish_current_child_root(&command)
        .await
        .expect("fresh same-command retry");
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn own_chain_wait_precedes_run_lock_and_serializes_duplicate_commands() {
    let n = runtime_fixture().await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let mut gate = n.f.w.db.pool.begin().await.expect("chain barrier");
    let pid = lock_chain(&mut gate, &n.f).await;
    let pool = n.f.w.execution_pool.clone();
    let first = command.clone();
    let first = tokio::spawn(async move {
        PgCapacityFinishStore::new(pool)
            .finish_current_child_root(&first)
            .await
    });
    let pool = n.f.w.execution_pool.clone();
    let second = command.clone();
    let second = tokio::spawn(async move {
        PgCapacityFinishStore::new(pool)
            .finish_current_child_root(&second)
            .await
    });
    waiting(&n.f, pid, "FROM execution_chains", 2).await;
    let mut probe = n.f.w.db.pool.begin().await.expect("own run remains free");
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE NOWAIT",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.w.namespace.id().as_uuid())
    .bind(head.as_uuid())
    .execute(&mut *probe)
    .await
    .expect("neither attempt locked run before chain");
    probe.rollback().await.expect("run probe ends");
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    gate.rollback().await.expect("release chain");
    let mut a = first.await.expect("first task").expect("first outcome");
    let mut b = second.await.expect("second task").expect("second outcome");
    assert_ne!(
        a.replayed, b.replayed,
        "one application and one serialized replay"
    );
    a.replayed = false;
    b.replayed = false;
    assert_eq!(a, b);
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
}

#[tokio::test]
async fn postwait_expiry_refuses_first_application_and_receipt_replay() {
    for replaying in [false, true] {
        let n = runtime_fixture().await;
        let head = complete_native_activity(&n).await;
        let mut command = native_command(&n, head).await;
        let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
        if replaying {
            store
                .finish_current_child_root(&command)
                .await
                .expect("initial receipt");
        }
        let before = execution_snapshot(&n.f).await;
        let counts = finish_counts(&n.f).await;
        let prior_state = state(&n.f.w.db.pool, &n.f).await;
        sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()+interval '2 seconds' WHERE shard=run_shard($1)")
            .bind(head.as_uuid()).execute(&n.f.w.db.pool).await.expect("live but soon-expiring own fence");
        let mut gate =
            n.f.w
                .db
                .pool
                .begin()
                .await
                .expect("unchanged shard tuple barrier");
        let expiry: time::OffsetDateTime = sqlx::query_scalar(
            "SELECT expires_at FROM scheduler_shards WHERE shard=run_shard($1) FOR UPDATE",
        )
        .bind(head.as_uuid())
        .fetch_one(&mut *gate)
        .await
        .expect("own shard held unchanged");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("gate pid");
        let pool = n.f.w.execution_pool.clone();
        let pending = command.clone();
        let pending = tokio::spawn(async move {
            PgCapacityFinishStore::new(pool)
                .finish_current_child_root(&pending)
                .await
        });
        waiting(&n.f, pid, "FROM scheduler_shards", 1).await;
        let still_live: bool = sqlx::query_scalar("SELECT clock_timestamp()<$1")
            .bind(expiry)
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("live-at-wait database time");
        assert!(
            still_live,
            "the unchanged-row wait must begin while the fence is still live"
        );
        eventually(
            "database clock passes committed expiry while shard row stays unchanged",
            || async {
                sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
                    .bind(expiry)
                    .fetch_one(&n.f.w.db.pool)
                    .await
                    .expect("database time")
                    .then_some(())
            },
        )
        .await;
        gate.rollback()
            .await
            .expect("unchanged tuple releases after expiry");
        assert_eq!(
            pending.await.expect("pending attempt"),
            Err(CapacityFinishError::SchedulerFenceLost)
        );
        assert_eq!(finish_counts(&n.f).await, counts);
        assert_eq!(state(&n.f.w.db.pool, &n.f).await, prior_state);
        assert_eq!(execution_snapshot(&n.f).await, before);
        let generation:i64=sqlx::query_scalar("UPDATE scheduler_shards SET generation=generation+1,owner='capacity-barrier-renewed',expires_at=clock_timestamp()+interval '1 hour' WHERE shard=run_shard($1) RETURNING generation")
            .bind(head.as_uuid()).fetch_one(&n.f.w.db.pool).await.expect("fresh fence");
        command.scheduler = SchedulerAuthority::for_run(
            "capacity-barrier-renewed",
            head,
            u64::try_from(generation).expect("generation"),
        );
        assert_eq!(
            store
                .finish_current_child_root(&command)
                .await
                .expect("fresh current authority")
                .replayed,
            replaying
        );
        assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    }
}

#[tokio::test]
async fn audit_outbox_receipt_and_deferred_commit_failures_roll_back_all_effects() {
    let n = runtime_fixture().await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let before = execution_snapshot(&n.f).await;
    sqlx::raw_sql("CREATE FUNCTION refuse_capacity_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected capacity write failure' USING ERRCODE='23514'; END $$;")
        .execute(&n.f.w.db.pool).await.expect("test-only failure function");
    for (create, drop) in [
        (
            "CREATE TRIGGER refuse_capacity_write BEFORE INSERT ON audit_records FOR EACH ROW WHEN(NEW.action='capacity.member.finished') EXECUTE FUNCTION refuse_capacity_write()",
            "DROP TRIGGER refuse_capacity_write ON audit_records",
        ),
        (
            "CREATE TRIGGER refuse_capacity_write BEFORE INSERT ON event_outbox FOR EACH ROW WHEN(NEW.event_type='capacity.member.finished') EXECUTE FUNCTION refuse_capacity_write()",
            "DROP TRIGGER refuse_capacity_write ON event_outbox",
        ),
        (
            "CREATE TRIGGER refuse_capacity_write BEFORE INSERT ON capacity_group_finish_receipts FOR EACH ROW EXECUTE FUNCTION refuse_capacity_write()",
            "DROP TRIGGER refuse_capacity_write ON capacity_group_finish_receipts",
        ),
        (
            "CREATE CONSTRAINT TRIGGER refuse_capacity_write AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_capacity_write()",
            "DROP TRIGGER refuse_capacity_write ON capacity_group_finish_receipts",
        ),
    ] {
        sqlx::raw_sql(create)
            .execute(&n.f.w.db.pool)
            .await
            .expect("install exact failure point");
        let result = PgCapacityFinishStore::new(n.f.w.execution_pool.clone())
            .finish_current_child_root(&command)
            .await;
        assert!(
            matches!(&result, Err(CapacityFinishError::Storage(sagat_application::error::StoreError::Internal { detail })) if detail.contains("injected capacity write failure")),
            "confirmed injected23514 must not be success/unknown: {result:?}"
        );
        assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
        assert_eq!(
            state(&n.f.w.db.pool, &n.f).await,
            (1, 1, 1, "HELD".into(), "ACTIVE".into())
        );
        assert_eq!(execution_snapshot(&n.f).await, before);
        sqlx::raw_sql(drop)
            .execute(&n.f.w.db.pool)
            .await
            .expect("remove fault");
    }
    sqlx::raw_sql("DROP FUNCTION refuse_capacity_write()")
        .execute(&n.f.w.db.pool)
        .await
        .expect("remove test function");
    PgCapacityFinishStore::new(n.f.w.execution_pool.clone())
        .finish_current_child_root(&command)
        .await
        .expect("same-command fresh successful retry");
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(execution_snapshot(&n.f).await, before);
}

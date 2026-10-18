//! Real lock barriers for the authority held by exact-path execution transactions.

use sagat_application::execution::{PathExecutionStore, PathExecutionTx, RunTx};
use sagat_application::runs::{Lock, RunStore};
use sagat_application::shards::Lease;
use sagat_domain::execution::InstancePath;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::NodeId;

use crate::scoped_worker::fixture;
use crate::support::eventually;

fn first_path() -> InstancePath {
    InstancePath::root()
        .child(NodeId::parse("items").expect("scope owner"), 0)
        .expect("member path")
}

#[tokio::test]
async fn an_existing_run_cannot_use_child_admission_authority() {
    let fixture = fixture("ordinary-run-not-child-admission").await;
    let shard = Shard::of(fixture.run.as_uuid());
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=$1")
            .bind(i16::from(shard.index()))
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("current run lease");
    let lease = Lease {
        shard,
        generation: u64::try_from(generation).expect("generation"),
    };
    let mut tx = fixture
        .store
        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
        .await
        .expect("ordinary transaction")
        .expect("run");
    assert!(
        !tx.holds_parent_shard(&owner, lease)
            .await
            .expect("not a new child")
    );
    assert!(
        tx.holds_shard(&owner, lease)
            .await
            .expect("normal own-shard authority remains valid")
    );
    tx.commit().await.expect("finish authority read");
}

#[tokio::test]
async fn a_bounded_binding_read_holds_current_authority_until_commit() {
    let fixture = fixture("bounded-binding-fence").await;
    let snapshot = fixture
        .store
        .load_path(fixture.world.admin.tenant_id(), fixture.run, first_path())
        .await
        .expect("exact-path snapshot")
        .expect("run");
    let mut tx = fixture
        .store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("bounded transaction")
        .expect("run");
    assert!(
        !tx.bound_patterns(fixture.world.worker.subject().as_str())
            .await
            .expect("current bindings")
            .is_empty()
    );
    let key = format!(
        "binding:{}:{}:{}",
        fixture.world.admin.tenant_id(),
        fixture.world.namespace.id(),
        fixture.world.worker.subject(),
    );
    let mut contender = fixture.world.db.pool.acquire().await.expect("revoker");
    sqlx::query("SET lock_timeout='100ms'")
        .execute(&mut *contender)
        .await
        .expect("bounded revocation wait");
    let blocked = sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&key)
        .execute(&mut *contender)
        .await
        .expect_err("the observed binding authority lasts through commit");
    assert_eq!(
        blocked
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03")
    );
    tx.commit().await.expect("commit releases authority");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&key)
        .execute(&mut *contender)
        .await
        .expect("revocation can proceed after the commit");
}

#[tokio::test]
async fn a_bounded_scheduler_fence_rechecks_expiry_after_its_row_lock_wait() {
    assert_expiry_after_shard_lock(true).await;
}

#[tokio::test]
async fn a_full_scheduler_fence_rechecks_expiry_after_its_row_lock_wait() {
    assert_expiry_after_shard_lock(false).await;
}

async fn assert_expiry_after_shard_lock(bounded: bool) {
    let fixture = fixture("scheduler-fence-clock").await;
    let shard = Shard::of(fixture.run.as_uuid());
    let (owner, generation): (String, i64) = sqlx::query_as(
        "UPDATE scheduler_shards SET expires_at=clock_timestamp()+interval '3 seconds' \
         WHERE shard=$1 RETURNING owner,generation",
    )
    .bind(i16::from(shard.index()))
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("visible lease expiry");
    let lease = Lease {
        shard,
        generation: u64::try_from(generation).expect("generation"),
    };
    let mut guard = fixture
        .world
        .db
        .pool
        .begin()
        .await
        .expect("shard lock guard");
    sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=$1 FOR UPDATE")
        .bind(i16::from(shard.index()))
        .execute(&mut *guard)
        .await
        .expect("hold shard row without modifying it");
    let store = fixture.store.clone();
    let tenant = fixture.world.admin.tenant_id();
    let run = fixture.run;
    let waiter = tokio::spawn(async move {
        if bounded {
            let snapshot = store
                .load_path(tenant, run, first_path())
                .await
                .expect("snapshot")
                .expect("run");
            let mut tx = store
                .begin_path(snapshot, Lock::Wait)
                .await
                .expect("bounded mutation")
                .expect("run");
            let held = tx.holds_shard(&owner, lease).await.expect("bounded fence");
            tx.commit().await.expect("finish bounded fence read");
            held
        } else {
            let mut tx = store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("full mutation")
                .expect("run");
            let held = tx.holds_shard(&owner, lease).await.expect("full fence");
            tx.commit().await.expect("finish full fence read");
            held
        }
    });
    eventually("the scheduler fence actually waits on the shard row", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
             AND state='active' AND wait_event_type='Lock' AND query LIKE '%FROM scheduler_shards%' \
             AND (NOT $1 OR usename='sagat_app'))",
        )
        .bind(crate::support::as_runtime_role())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("database lock barrier")
        .then_some(())
    })
    .await;
    eventually(
        "the visible scheduler lease expires while its row stays locked",
        || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT clock_timestamp()>=expires_at FROM scheduler_shards WHERE shard=$1",
            )
            .bind(i16::from(shard.index()))
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("database expiry barrier")
            .then_some(())
        },
    )
    .await;
    guard.rollback().await.expect("release unchanged shard row");
    assert!(
        !waiter.await.expect("fence waiter"),
        "waiting for a row lock must not authorize an already expired shard lease"
    );
}

#[tokio::test]
async fn chain_prefix_current_path_skips_before_locking_its_run() {
    assert_chain_prefix(true, true).await;
}

#[tokio::test]
async fn chain_prefix_snapshot_path_skips_before_locking_its_run() {
    assert_chain_prefix(false, true).await;
}

#[tokio::test]
async fn chain_prefix_current_path_waits_without_holding_its_run() {
    assert_chain_prefix(true, false).await;
}

#[tokio::test]
async fn chain_prefix_snapshot_path_waits_without_holding_its_run() {
    assert_chain_prefix(false, false).await;
}

async fn assert_chain_prefix(current: bool, skip: bool) {
    let fixture = fixture("bounded-chain-lock-prefix").await;
    let tenant = fixture.world.admin.tenant_id();
    let run = fixture.run;
    // Storage protocol fixture only: public chain+collection composition is still closed.
    // Use the real initializer, with its deadline and journal revision, before loading a snapshot.
    let mut setup = fixture.world.db.pool.begin().await.expect("chain setup");
    sqlx::query(
        "UPDATE runs SET deadline_at=clock_timestamp()+interval '1 hour' WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .execute(&mut *setup)
    .await
    .expect("chain deadline");
    sagat_infra_postgres::initialize_chain_in_transaction(
        &mut setup,
        &sagat_application::chain_progress::ChainInitialization::new(
            sagat_domain::execution::ChainScope::new(tenant, fixture.world.namespace.id()),
            run,
            sagat_domain::execution::ChainPolicy::new(2).expect("chain policy"),
        ),
    )
    .await
    .expect("initialize singleton chain");
    setup.commit().await.expect("consistent singleton metadata");
    let snapshot = fixture
        .store
        .load_path(tenant, run, first_path())
        .await
        .expect("unlocked scoped snapshot")
        .expect("scoped run");
    let mut guard = fixture.world.db.pool.begin().await.expect("chain guard");
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("guard backend");
    sqlx::query(
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .execute(&mut *guard)
    .await
    .expect("hold only the chain");
    let store = fixture.store.clone();
    let waiter = tokio::spawn(async move {
        let lock = if skip { Lock::SkipLocked } else { Lock::Wait };
        if current {
            store
                .begin_current_path(tenant, run, first_path(), lock)
                .await
        } else {
            store.begin_path(snapshot, lock).await
        }
    });
    if skip {
        let tx = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("skip must not wait")
            .expect("skip task")
            .expect("skip result");
        assert!(tx.is_none(), "a contended chain skips before the run lock");
    } else {
        eventually("the exact-path transaction waits on its chain", || async {
            assert!(
                !waiter.is_finished(),
                "the chain guard must block the path entry"
            );
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%FROM execution_chains%' \
                 AND (NOT $2 OR usename='sagat_app'))",
            )
            .bind(blocker)
            .bind(crate::support::as_runtime_role())
            .fetch_one(&fixture.world.db.pool)
            .await
            .expect("chain wait barrier")
            .then_some(())
        })
        .await;
        let mut probe = fixture.world.db.pool.begin().await.expect("run lock probe");
        sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .execute(&mut *probe)
            .await
            .expect("waiting for the chain must not hold the run");
        probe.rollback().await.expect("release run probe");
        guard.rollback().await.expect("release chain barrier");
        let tx = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("path proceeds after the chain")
            .expect("wait task")
            .expect("path result")
            .expect("path transaction");
        // The guard belongs to this mutation transaction, not a short-lived preparatory one.
        let mut probe = fixture
            .world
            .db
            .pool
            .acquire()
            .await
            .expect("held lock probe");
        for statement in [
            "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE NOWAIT",
            "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT",
        ] {
            let blocked = sqlx::query(statement)
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .execute(&mut *probe)
                .await
                .expect_err("path transaction keeps both locks until commit");
            assert_eq!(
                blocked
                    .as_database_error()
                    .and_then(|e| e.code())
                    .as_deref(),
                Some("55P03")
            );
        }
        drop(probe);
        tx.commit().await.expect("commit releases both locks");
        let mut probe = fixture
            .world
            .db
            .pool
            .begin()
            .await
            .expect("released lock probe");
        for statement in [
            "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE NOWAIT",
            "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT",
        ] {
            sqlx::query(statement)
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .execute(&mut *probe)
                .await
                .expect("commit releases chain and run");
        }
        probe.rollback().await.expect("finish release probe");
        return;
    }
    // A skip did not take the run lock either, and dropping its transaction did not leak a lock.
    let mut probe = fixture.world.db.pool.begin().await.expect("skip run probe");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .execute(&mut *probe)
        .await
        .expect("skipped chain leaves the run unlocked");
    probe.rollback().await.expect("finish skip probe");
    guard.rollback().await.expect("release chain guard");
}

/// Holds an unchanged shard tuple while a real runtime transaction waits on its SHARE guard.
/// The tuple is released only after its persisted expiry passes on the database clock.
pub(super) struct SchedulerExpiryBarrier {
    guard: sqlx::Transaction<'static, sqlx::Postgres>,
    pool: sqlx::PgPool,
    backend: i32,
    expires_at: time::OffsetDateTime,
}

impl SchedulerExpiryBarrier {
    pub(super) async fn hold(pool: &sqlx::PgPool, run: sagat_domain::execution::RunId) -> Self {
        let expires_at = sqlx::query_scalar(
            "UPDATE scheduler_shards SET expires_at=clock_timestamp()+interval '3 seconds' \
             WHERE shard=run_shard($1) RETURNING expires_at",
        )
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("persist short current-authority expiry");
        let mut guard = pool.begin().await.expect("unchanged shard guard");
        let backend = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *guard)
            .await
            .expect("shard guard backend");
        sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR UPDATE")
            .bind(run.as_uuid())
            .execute(&mut *guard)
            .await
            .expect("hold unchanged shard tuple");
        Self {
            guard,
            pool: pool.clone(),
            backend,
            expires_at,
        }
    }

    pub(super) async fn expire_after_wait(self) {
        eventually(
            "runtime authority waits on the unchanged shard tuple",
            || async {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%FROM scheduler_shards%' \
                 AND (NOT $2 OR usename='sagat_app'))",
                )
                .bind(self.backend)
                .bind(crate::support::as_runtime_role())
                .fetch_one(&self.pool)
                .await
                .expect("actual scheduler guard wait")
                .then_some(())
            },
        )
        .await;
        let still_live: bool = sqlx::query_scalar("SELECT clock_timestamp()<$1")
            .bind(self.expires_at)
            .fetch_one(&self.pool)
            .await
            .expect("authority was live when it began waiting");
        assert!(still_live, "the wait must begin before expiry");
        eventually(
            "the persisted lease expires under the unchanged tuple guard",
            || async {
                sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
                    .bind(self.expires_at)
                    .fetch_one(&self.pool)
                    .await
                    .expect("database expiry barrier")
                    .then_some(())
            },
        )
        .await;
        self.guard
            .rollback()
            .await
            .expect("release unchanged tuple after expiry");
    }
}

/// Independent persisted effects, excluding the deliberately changed scheduler lease itself.
pub(super) async fn chain_checkpoint(
    world: &crate::support::engine::World,
    chain: sagat_domain::execution::RunId,
) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
         'chain',(SELECT to_jsonb(c) FROM execution_chains c WHERE c.tenant_id=$1 AND c.chain_id=$2), \
         'segments',(SELECT jsonb_agg(to_jsonb(s) ORDER BY s.segment_index) FROM chain_segments s WHERE s.tenant_id=$1 AND s.chain_id=$2), \
         'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM runs r WHERE r.tenant_id=$1), \
         'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.run_id,e.seq) FROM run_events e WHERE e.tenant_id=$1), \
         'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM event_outbox o WHERE o.tenant_id=$1), \
         'audit',(SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM audit_records a WHERE a.tenant_id=$1), \
         'usage',(SELECT to_jsonb(u) FROM tenant_retained_usage u WHERE u.tenant_id=$1))",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("independent chain effects checkpoint")
}

/// One independent snapshot of the persisted effects owned by collection/loop/approval transitions.
pub(super) async fn scope_checkpoint(pool: &sqlx::PgPool, tenant: uuid::Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
         'runs',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM runs t WHERE t.tenant_id=$1), \
         'telemetry_pending',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM telemetry_pending t WHERE t.tenant_id=$1), \
         'approval_requests',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM approval_requests t WHERE t.tenant_id=$1), \
         'approval_decision_receipts',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM approval_decision_receipts t WHERE t.tenant_id=$1), \
         'approval_timers',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM approval_timers t WHERE t.tenant_id=$1), \
         'steps',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM steps t WHERE t.tenant_id=$1), \
         'run_events',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM run_events t WHERE t.tenant_id=$1), \
         'event_outbox',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM event_outbox t WHERE t.tenant_id=$1), \
         'audit_records',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM audit_records t WHERE t.tenant_id=$1), \
         'payloads',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM payloads t WHERE t.tenant_id=$1), \
         'iteration_scopes',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM iteration_scopes t WHERE t.tenant_id=$1), \
         'collection_scope_items',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM collection_scope_items t WHERE t.tenant_id=$1), \
         'collection_iterations',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM collection_iterations t WHERE t.tenant_id=$1), \
         'iteration_checkpoints',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM iteration_checkpoints t WHERE t.tenant_id=$1), \
         'loop_scopes',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM loop_scopes t WHERE t.tenant_id=$1), \
         'loop_iterations',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM loop_iterations t WHERE t.tenant_id=$1), \
         'loop_checkpoints',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM loop_checkpoints t WHERE t.tenant_id=$1), \
         'loop_timers',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM loop_timers t WHERE t.tenant_id=$1), \
         'run_instance_usage',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM run_instance_usage t WHERE t.tenant_id=$1), \
         'tenant_iteration_usage',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM tenant_iteration_usage t WHERE t.tenant_id=$1), \
         'tenant_retained_usage',(SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM tenant_retained_usage t WHERE t.tenant_id=$1))",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("independent scope effects checkpoint")
}

/// Storage-only setup; supported public compositions are not changed by these tests.
pub(super) async fn initialize_member_chain(
    pool: &sqlx::PgPool,
    tenant: sagat_domain::identity::TenantId,
    namespace: sagat_domain::identity::NamespaceId,
    run: sagat_domain::execution::RunId,
) {
    let mut tx = pool.begin().await.expect("member chain setup");
    sqlx::query("UPDATE runs SET deadline_at=clock_timestamp()+interval '1 hour' WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid()).bind(run.as_uuid()).execute(&mut *tx).await.expect("chain deadline");
    sagat_infra_postgres::initialize_chain_in_transaction(
        &mut tx,
        &sagat_application::chain_progress::ChainInitialization::new(
            sagat_domain::execution::ChainScope::new(tenant, namespace),
            run,
            sagat_domain::execution::ChainPolicy::new(2).expect("chain policy"),
        ),
    )
    .await
    .expect("initialize real member chain");
    tx.commit().await.expect("atomic chain initialization");
}

/// Proves the prefix, not an independent near-commit lifetime oracle for every primitive.
pub(super) async fn through_member_chain<T: std::fmt::Debug>(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    run: sagat_domain::execution::RunId,
    mutation: impl std::future::Future<Output = T>,
) -> T {
    let mut guard = pool.begin().await.expect("member chain guard");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("chain guard backend");
    sqlx::query(
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&mut *guard)
    .await
    .expect("hold member chain");
    let mut mutation = Box::pin(mutation);
    tokio::select! {
        result = &mut mutation => panic!("primitive bypassed its member chain: {result:?}"),
        () = eventually("primitive waits on chain before own run", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%execution_chains%' \
                 AND (NOT $2 OR usename='sagat_app'))",
            ).bind(backend).bind(crate::support::as_runtime_role()).fetch_one(pool).await
            .expect("actual primitive chain wait").then_some(())
        }) => {}
    }
    let mut probe = pool.acquire().await.expect("primitive early run probe");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
        .bind(tenant)
        .bind(run.as_uuid())
        .execute(&mut *probe)
        .await
        .expect("primitive waits on chain without retaining its own run");
    drop(probe);
    guard.rollback().await.expect("release member chain");
    mutation.await
}

#[tokio::test]
async fn session_room_post_wait_full_rejects_expired_authority() {
    assert_session_room_post_wait(false).await;
}

#[tokio::test]
async fn session_room_post_wait_path_rejects_expired_authority() {
    assert_session_room_post_wait(true).await;
}

async fn assert_session_room_post_wait(bounded: bool) {
    use sagat_application::entropy::Token;
    use sagat_application::worker::Capabilities;

    let fixture = fixture("session-room-post-wait").await;
    let pool = &fixture.world.db.pool;
    let tenant = fixture.world.admin.tenant_id();
    let id = fixture.session.worker_id;
    let expires_at: time::OffsetDateTime = sqlx::query_scalar(
        "UPDATE worker_sessions SET expires_at=clock_timestamp()+interval '3 seconds' \
         WHERE tenant_id=$1 AND id=$2 RETURNING expires_at",
    )
    .bind(tenant.as_uuid())
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("short session expiry");
    let before = scope_checkpoint(pool, tenant.as_uuid()).await;
    let mut guard = pool.begin().await.expect("unchanged session guard");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("session guard backend");
    sqlx::query("SELECT id FROM worker_sessions WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant.as_uuid())
        .bind(id)
        .execute(&mut *guard)
        .await
        .expect("lock unchanged session");
    let store = fixture.store.clone();
    let run = fixture.run;
    let hash = Token::hash_of(&fixture.session.token);
    let subject = fixture.session.principal.subject().as_str().to_owned();
    // Exercise the claim-time port directly, after session creation. Public poll first renews
    // the session; blocking that earlier renewal would not test either claim-time guard.
    let waiter = tokio::spawn(async move {
        if bounded {
            let mut tx = store
                .begin_current_path(tenant, run, first_path(), Lock::Wait)
                .await
                .expect("path claim transaction")
                .expect("run");
            let room = tx
                .worker_session_room(id, &hash, &subject)
                .await
                .expect("path session room");
            tx.commit().await.expect("commit path authority read");
            room
        } else {
            let mut tx = store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("full claim transaction")
                .expect("run");
            let room = tx
                .worker_session_room(id, &hash, &subject)
                .await
                .expect("full session room");
            tx.commit().await.expect("commit full authority read");
            room
        }
    });
    eventually("claim session guard actually waits while live", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
             AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%FROM worker_sessions%' \
             AND (NOT $2 OR usename='sagat_app'))",
        )
        .bind(backend)
        .bind(crate::support::as_runtime_role())
        .fetch_one(pool)
        .await
        .expect("actual session room guard wait")
        .then_some(())
    })
    .await;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()<$1")
            .bind(expires_at)
            .fetch_one(pool)
            .await
            .expect("session live at wait")
    );
    eventually(
        "session expires while its tuple stays unchanged",
        || async {
            sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
                .bind(expires_at)
                .fetch_one(pool)
                .await
                .expect("database expiry")
                .then_some(())
        },
    )
    .await;
    guard.rollback().await.expect("release expired session");
    assert_eq!(
        waiter.await.expect("claim room waiter"),
        0,
        "session expired while waiting cannot supply claim capacity"
    );
    assert_eq!(
        scope_checkpoint(pool, tenant.as_uuid()).await,
        before,
        "refused capacity cannot change execution effects"
    );
    let unchanged_expiry: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM worker_sessions WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid())
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("session not renewed by room check");
    assert_eq!(unchanged_expiry, expires_at);

    let fresh = fixture
        .workers
        .register(
            &fixture.world.worker,
            "payments",
            &format!("room-positive-{}", uuid::Uuid::now_v7()),
            &Capabilities {
                activities: vec!["scoped.a".into(), "scoped.b".into()],
                languages: vec!["rust".into()],
                pools: vec![],
                max_inflight: 8,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("fresh registered claim authority");
    let hash = fresh.session_token.hash();
    let subject = fixture.world.worker.subject().as_str();
    if bounded {
        let mut tx = fixture
            .store
            .begin_current_path(tenant, run, first_path(), Lock::Wait)
            .await
            .expect("fresh path transaction")
            .expect("run");
        assert_eq!(
            tx.worker_session_room(fresh.worker_id, &hash, subject)
                .await
                .expect("live room"),
            8
        );
        assert_eq!(
            tx.worker_session_room(id, &Token::hash_of(&fixture.session.token), subject)
                .await
                .expect("old room remains refused"),
            0
        );
        tx.commit().await.expect("commit fresh path check");
    } else {
        let mut tx = fixture
            .store
            .begin(tenant, run, Lock::Wait)
            .await
            .expect("fresh full transaction")
            .expect("run");
        assert_eq!(
            tx.worker_session_room(fresh.worker_id, &hash, subject)
                .await
                .expect("live room"),
            8
        );
        assert_eq!(
            tx.worker_session_room(id, &Token::hash_of(&fixture.session.token), subject)
                .await
                .expect("old room remains refused"),
            0
        );
        tx.commit().await.expect("commit fresh full check");
    }
    fixture
        .store
        .verify_run(tenant, run)
        .await
        .expect("execution projection stays consistent");
}

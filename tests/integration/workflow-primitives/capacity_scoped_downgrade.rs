//! Conditional downgrade evidence for the dormant fixed scoped Finish binding.
//! Raw scoped rows in this module prove only the migration guard and lock protocol.

use super::super::{
    Fixture, complete_native_activity, error_is, fixture, native_command, native_fixture,
};
use super::{INDEX, OWNER};
use sagat_application::capacity_finish::CurrentChildRootFinishStore;
use sagat_infra_postgres::{MIGRATOR, PgCapacityFinishStore, migrate};
use sqlx::{PgPool, migrate::MigrateError};
use uuid::Uuid;

const BEFORE_SCOPED_FINISH: i64 = 20_261_211_000_000;
const SCOPED_FINISH_VERSION: i64 = 20_261_212_000_000;

/// The newest migration of the combined profile: later migrations (the collection data-limit
/// drain, 20261213000000) sit above the scoped Finish extension this module exercises.
fn head_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .expect("migrations")
}

fn retained_scoped_downgrade(error: MigrateError) {
    match error {
        MigrateError::ExecuteMigration(error, version) => {
            assert_eq!(version, SCOPED_FINISH_VERSION);
            error_is(error, "55000", None);
        }
        other => panic!("expected typed retained-scoped downgrade refusal, got {other:?}"),
    }
}

async fn legacy_capacity_snapshot(pool: &PgPool, f: &Fixture) -> (String, String, String) {
    sqlx::query_as(
        "SELECT \
         (SELECT md5(COALESCE(jsonb_agg(jsonb_build_array(\
             group_id,revision,epoch,active_members,possession,holder_member_id) \
             ORDER BY group_id),'[]'::jsonb)::text) \
          FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2),\
         (SELECT md5(COALESCE(jsonb_agg(jsonb_build_array(\
             group_id,member_id,namespace_id,member_kind,chain_id,state,finished_at) \
             ORDER BY member_id),'[]'::jsonb)::text) \
          FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2),\
         (SELECT md5(COALESCE(jsonb_agg(jsonb_build_array(\
             group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,\
             expected_group_revision,expected_group_epoch,accepted_chain_revision,\
             accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,\
             resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id) \
             ORDER BY operation_key),'[]'::jsonb)::text) \
          FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .fetch_one(pool)
    .await
    .expect("legacy capacity rows snapshot")
}

async fn scoped_column_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema=current_schema() AND (\
           (table_name='capacity_group_members' AND \
            column_name IN ('original_run_id','root_owner_node','iteration_index')) OR\
           (table_name='capacity_group_finish_receipts' AND \
            column_name IN ('member_kind','original_run_id','root_owner_node','iteration_index')))",
    )
    .fetch_one(pool)
    .await
    .expect("scoped extension column catalog")
}

async fn migration_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
        .expect("current migration version")
}

async fn receipt_insert_grants(pool: &PgPool) -> (bool, bool, bool) {
    sqlx::query_as(
        "SELECT \
         has_table_privilege(current_user,'capacity_group_finish_receipts','INSERT'),\
         has_column_privilege(current_user,'capacity_group_finish_receipts','tenant_id','INSERT'),\
         CASE WHEN EXISTS (\
             SELECT 1 FROM pg_attribute \
             WHERE attrelid='capacity_group_finish_receipts'::regclass \
               AND attname='original_run_id' AND NOT attisdropped)\
         THEN has_column_privilege(\
             current_user,'capacity_group_finish_receipts','original_run_id','INSERT')\
         ELSE false END",
    )
    .fetch_one(pool)
    .await
    .expect("effective receipt INSERT grants")
}

#[tokio::test]
async fn child_root_native_finish_survives_down_and_up_with_legacy_grants() {
    let n = native_fixture(false).await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let runtime = n.f.w.db.runtime_pool().await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("actual runtime role for native apply/replay");
    assert_eq!(role, "sagat_app");
    let store = PgCapacityFinishStore::new(runtime.clone());
    let applied = store
        .finish_current_child_root(&command)
        .await
        .expect("actual native ChildRoot Finish applies");
    assert!(!applied.replayed);
    let core = legacy_capacity_snapshot(&n.f.w.db.pool, &n.f).await;

    MIGRATOR
        .undo(&n.f.w.db.pool, BEFORE_SCOPED_FINISH)
        .await
        .expect("ChildRoot-only storage downgrades");
    assert_eq!(
        migration_version(&n.f.w.db.pool).await,
        BEFORE_SCOPED_FINISH
    );
    assert_eq!(scoped_column_count(&n.f.w.db.pool).await, 0);
    assert_eq!(legacy_capacity_snapshot(&n.f.w.db.pool, &n.f).await, core);
    assert_eq!(
        receipt_insert_grants(&runtime).await,
        (true, true, false),
        "legacy table-level receipt INSERT is restored while down"
    );
    let mut replay = store
        .finish_current_child_root(&command)
        .await
        .expect("exact native receipt replays against the legacy schema");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(replay, applied);

    migrate(&n.f.w.db.pool)
        .await
        .expect("scoped Finish extension migrates up again");
    assert_eq!(migration_version(&n.f.w.db.pool).await, head_version());
    assert_eq!(scoped_column_count(&n.f.w.db.pool).await, 7);
    assert_eq!(legacy_capacity_snapshot(&n.f.w.db.pool, &n.f).await, core);
    assert_eq!(
        receipt_insert_grants(&runtime).await,
        (false, true, false),
        "upgrade restores the exact legacy-column grant"
    );
    let mut replay = store
        .finish_current_child_root(&command)
        .await
        .expect("exact native receipt still replays after re-upgrade");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(replay, applied);
}

#[derive(Clone, Copy)]
enum RetainedScopedKind {
    ActiveMember,
    FinishedMemberAndReceipt,
}

struct RetainedScopedFixture {
    base: Fixture,
    member: Uuid,
    original_run: Uuid,
}

async fn seed_retained_scoped(kind: RetainedScopedKind) -> RetainedScopedFixture {
    let base = fixture().await;
    let (workflow, _) = base
        .w
        .publish(crate::support::engine::chain(&format!(
            "scoped-down-original-{}",
            Uuid::new_v4()
        )))
        .await;
    let original_run = base
        .w
        .start(workflow, &serde_json::json!({}))
        .await
        .as_uuid();
    let member = Uuid::new_v4();
    let tenant = base.w.admin.tenant_id().as_uuid();
    let namespace = base.w.namespace.id().as_uuid();
    let mut tx = base
        .w
        .db
        .pool
        .begin()
        .await
        .expect("scoped retention fixture");

    let _: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(original_run)
    .fetch_one(&mut *tx)
    .await
    .expect("retain plain original Run first");
    let _: Uuid = sqlx::query_scalar(
        "SELECT group_id FROM capacity_groups \
         WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(base.group)
    .fetch_one(&mut *tx)
    .await
    .expect("retain selected capacity group second");

    let (state, active_members): (&str, i64) = match kind {
        RetainedScopedKind::ActiveMember => ("ACTIVE", 2),
        RetainedScopedKind::FinishedMemberAndReceipt => ("FINISHED", 1),
    };
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=$3 \
         WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(tenant)
    .bind(base.group)
    .bind(active_members)
    .execute(&mut *tx)
    .await
    .expect("raw scoped fixture header");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,finished_at) \
         VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,$8,\
           CASE WHEN $8='FINISHED' THEN clock_timestamp() ELSE NULL END)",
    )
    .bind(tenant)
    .bind(base.group)
    .bind(member)
    .bind(namespace)
    .bind(original_run)
    .bind(OWNER)
    .bind(INDEX)
    .bind(state)
    .execute(&mut *tx)
    .await
    .expect("raw valid scoped member");

    if matches!(kind, RetainedScopedKind::FinishedMemberAndReceipt) {
        sqlx::query(
            "INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,\
             fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,\
             expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,\
             resulting_epoch,resulting_active_members,resulting_possession,\
             resulting_holder_member_id,finished_at,audit_id,outbox_id,member_kind,original_run_id,\
             root_owner_node,iteration_index) \
             VALUES($1,$2,$3,$4,$5,$6,NULL,NULL,1,1,NULL,1,2,1,1,'HELD',$7,\
             clock_timestamp(),$8,$9,'SCOPED_PATH',$10,$11,$12)",
        )
        .bind(tenant)
        .bind(base.group)
        .bind(Uuid::new_v4())
        .bind(vec![9_u8; 32])
        .bind(namespace)
        .bind(member)
        .bind(base.member)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(original_run)
        .bind(OWNER)
        .bind(INDEX)
        .execute(&mut *tx)
        .await
        .expect("raw valid scoped receipt");
    }
    tx.commit().await.expect("scoped retention fixture commits");
    RetainedScopedFixture {
        base,
        member,
        original_run,
    }
}

async fn retained_scoped_state(f: &RetainedScopedFixture) -> (i64, i64, i64, i64, String, Uuid) {
    sqlx::query_as(
        "SELECT \
         (SELECT max(version) FROM _sqlx_migrations WHERE success),\
         g.revision,g.active_members,\
         (SELECT count(*) FROM capacity_group_finish_receipts r \
          WHERE r.tenant_id=g.tenant_id AND r.group_id=g.group_id AND r.member_id=m.member_id),\
         m.state,m.original_run_id \
         FROM capacity_groups g JOIN capacity_group_members m USING(tenant_id,group_id) \
         WHERE g.tenant_id=$1 AND g.group_id=$2 AND m.member_id=$3 \
           AND m.member_kind='SCOPED_PATH' AND m.root_owner_node=$4 AND m.iteration_index=$5",
    )
    .bind(f.base.w.admin.tenant_id().as_uuid())
    .bind(f.base.group)
    .bind(f.member)
    .bind(OWNER)
    .bind(INDEX)
    .fetch_one(&f.base.w.db.pool)
    .await
    .expect("retained scoped rows and migration version")
}

#[tokio::test]
async fn active_member_and_finished_tombstone_receipt_each_refuse_downgrade() {
    for kind in [
        RetainedScopedKind::ActiveMember,
        RetainedScopedKind::FinishedMemberAndReceipt,
    ] {
        let f = seed_retained_scoped(kind).await;
        let before = retained_scoped_state(&f).await;
        assert_eq!(before.0, head_version());
        assert_eq!(before.5, f.original_run);
        retained_scoped_downgrade(
            MIGRATOR
                .undo(&f.base.w.db.pool, BEFORE_SCOPED_FINISH)
                .await
                .expect_err("every retained scoped variant blocks reversal"),
        );
        // Undo runs newest first: the data-limit drain migration (20261213000000) is reversed in its
        // own step before the scoped Finish extension refuses, so only the version differs.
        let mut expected = before.clone();
        expected.0 = SCOPED_FINISH_VERSION;
        assert_eq!(retained_scoped_state(&f).await, expected);
    }
}

#[tokio::test]
async fn downgrade_waits_on_original_run_then_observes_committed_scoped_member() {
    let f = fixture().await;
    let (workflow, _) =
        f.w.publish(crate::support::engine::chain(&format!(
            "scoped-down-writer-{}",
            Uuid::new_v4()
        )))
        .await;
    let original_run = f.w.start(workflow, &serde_json::json!({})).await.as_uuid();
    // Isolate the scoped Finish down migration's Run-first protocol. Later independent
    // index reversals may legitimately wait on their own relation before this migration starts.
    MIGRATOR
        .undo(&f.w.db.pool, SCOPED_FINISH_VERSION)
        .await
        .expect("remove later migrations before establishing this migration's writer barrier");
    assert_eq!(migration_version(&f.w.db.pool).await, SCOPED_FINISH_VERSION);

    let runtime = f.w.db.runtime_pool().await;
    let member = Uuid::new_v4();
    let tenant = f.w.admin.tenant_id().as_uuid();
    let namespace = f.w.namespace.id().as_uuid();
    let mut writer = f.w.db.pool.begin().await.expect("actual scoped writer");
    let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .expect("writer backend pid");

    let _: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(original_run)
    .fetch_one(&mut *writer)
    .await
    .expect("writer retains original Run first");
    let _: Uuid = sqlx::query_scalar(
        "SELECT group_id FROM capacity_groups \
         WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(f.group)
    .fetch_one(&mut *writer)
    .await
    .expect("writer retains G second");
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=2 \
         WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(tenant)
    .bind(f.group)
    .execute(&mut *writer)
    .await
    .expect("uncommitted valid scoped header");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state) \
         VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,'ACTIVE')",
    )
    .bind(tenant)
    .bind(f.group)
    .bind(member)
    .bind(namespace)
    .bind(original_run)
    .bind(OWNER)
    .bind(INDEX)
    .execute(&mut *writer)
    .await
    .expect("uncommitted valid scoped member");

    let pool = f.w.db.pool.clone();
    let downgrade = tokio::spawn(async move { MIGRATOR.undo(&pool, BEFORE_SCOPED_FINISH).await });
    crate::support::eventually(
        "down migration has an ungranted runs AccessExclusiveLock blocked by the writer",
        || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(\
                   SELECT 1 FROM pg_locks waiting \
                   JOIN pg_stat_activity activity ON activity.pid=waiting.pid \
                   WHERE waiting.locktype='relation' \
                     AND waiting.relation='runs'::regclass \
                     AND waiting.mode='AccessExclusiveLock' \
                     AND NOT waiting.granted \
                     AND activity.datname=current_database() \
                     AND $1=ANY(pg_blocking_pids(waiting.pid)))",
            )
            .bind(writer_pid)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("explicit runs relation lock wait")
            .then_some(())
        },
    )
    .await;

    writer.commit().await.expect("scoped writer commits first");
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), downgrade)
        .await
        .expect("downgrade leaves the lock wait without a cycle")
        .expect("downgrade task")
        .expect_err("committed scoped member is visible to the guard");
    retained_scoped_downgrade(error);

    let preserved: (i64, i64, i64, String, Uuid, String, i32) = sqlx::query_as(
        "SELECT \
         (SELECT max(version) FROM _sqlx_migrations WHERE success),\
         g.revision,g.active_members,m.state,m.original_run_id,m.root_owner_node,m.iteration_index \
         FROM capacity_groups g JOIN capacity_group_members m USING(tenant_id,group_id) \
         WHERE g.tenant_id=$1 AND g.group_id=$2 AND m.member_id=$3",
    )
    .bind(tenant)
    .bind(f.group)
    .bind(member)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("committed scoped row survives refused downgrade");
    assert_eq!(
        preserved,
        (
            SCOPED_FINISH_VERSION,
            2,
            2,
            "ACTIVE".into(),
            original_run,
            OWNER.into(),
            INDEX,
        )
    );
    let guards: (i64, bool) = sqlx::query_as(
        "SELECT count(*),bool_and(tgenabled='A') FROM pg_trigger \
         WHERE tgrelid IN ('capacity_group_members'::regclass,\
             'capacity_group_finish_receipts'::regclass) \
           AND tgname IN ('capacity_group_members_guard','capacity_group_members_no_truncate',\
             'capacity_group_finish_receipts_append_only',\
             'capacity_group_finish_receipts_no_truncate')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("scoped retention guards survive refused downgrade");
    assert_eq!(guards, (4, true));
    assert_eq!(
        receipt_insert_grants(&runtime).await,
        (false, true, false),
        "refused downgrade preserves the narrow runtime grant"
    );
}

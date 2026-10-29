//! Dormant capacity metadata only. Owner fixtures establish referential shape, not production
//! ChildRoot admission, physical child authority, quota reservation or execution eligibility.

use crate::support::engine::{World, chain, world};
use sagat_domain::execution::RunId;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

struct Fixture {
    w: World,
    group: Uuid,
    member: Uuid,
    chain: Uuid,
}

async fn seed_chain(w: &World, name: &str) -> RunId {
    let (workflow, version) = w.publish(chain(name)).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let mut tx = w.db.pool.begin().await.expect("chain fixture transaction");
    sqlx::query(
        "INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,\
         definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) \
         VALUES($1,$2,$3,$3,$4,$5,2,$6,$7,$7)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(version.as_uuid())
    .bind(w.admin.subject().as_str())
    .bind(crate::support::now() + time::Duration::hours(1))
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("existing chain identity fixture");
    sqlx::query(
        "INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) \
         VALUES($1,$2,$3,0,$3,$4)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(crate::support::now())
    .execute(&mut *tx)
    .await
    .expect("existing segment fixture");
    tx.commit().await.expect("chain fixture commits");
    run
}

async fn fixture() -> Fixture {
    let w = world().await;
    let run = seed_chain(&w, "capacity-schema").await;
    seed_capacity(w, run).await
}

async fn seed_capacity(w: World, run: RunId) -> Fixture {
    let group = Uuid::new_v4();
    let member = Uuid::new_v4();
    let mut tx = w.db.pool.begin().await.expect("owner capacity fixture");
    sqlx::query(
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) \
         VALUES($1,$2,1,1,1,'HELD',$3)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(group)
    .bind(member)
    .execute(&mut *tx)
    .await
    .expect("owner-only dormant group fixture");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) \
         VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(group)
    .bind(member)
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("owner-only dormant binding fixture");
    tx.commit().await.expect("deferred active holder resolves");
    Fixture {
        w,
        group,
        member,
        chain: run.as_uuid(),
    }
}

async fn state(pool: &PgPool, f: &Fixture) -> (i64, i64, i64, String, String) {
    sqlx::query_as(
        "SELECT g.revision,g.epoch,g.active_members,g.possession,m.state \
         FROM capacity_groups g JOIN capacity_group_members m USING(tenant_id,group_id) \
         WHERE g.tenant_id=$1 AND g.group_id=$2 AND m.member_id=$3",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .bind(f.member)
    .fetch_one(pool)
    .await
    .expect("exact fixture state")
}

#[tokio::test]
async fn valid_owner_fixture_precedes_capacity_constraint_probes() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool().await;
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("actual runtime identity");
    assert_eq!(user, "sagat_app");
    assert_eq!(
        state(&runtime, &f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
}

fn error_is(error: sqlx::Error, code: &str, constraint: Option<&str>) {
    let (actual, name) = crate::support::db_error(error);
    assert_eq!(actual, code);
    if let Some(expected) = constraint {
        assert_eq!(name.as_deref(), Some(expected));
    }
}

async fn finish_member(conn: &mut PgConnection, f: &Fixture, member: Uuid) {
    sqlx::query("UPDATE capacity_group_members SET state='FINISHED',finished_at=clock_timestamp() WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(member)
        .execute(conn).await.expect("finish fixture member");
}

async fn retire(conn: &mut PgConnection, f: &Fixture) {
    sqlx::query("UPDATE capacity_groups SET revision=revision+1,epoch=epoch+1,active_members=0,possession='RETIRED',holder_member_id=NULL WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group)
        .execute(conn).await.expect("retire fixture header");
}

// Raw schema fixture, deliberately not the native fenced Finish operation.
async fn receipt(
    conn: &mut PgConnection,
    f: &Fixture,
    member: Uuid,
    chain: Uuid,
    expected: i64,
    result: (i64, &str, Option<Uuid>, i64),
) -> Result<(), sqlx::Error> {
    let (epoch, possession, holder, count) = result;
    sqlx::query("INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id) VALUES($1,$2,$3,$4,$5,$6,$7,$7,$8,1,0,0,$8+1,$9,$10,$11,$12,clock_timestamp(),$13,$14)")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(Uuid::new_v4())
        .bind(vec![7_u8;32]).bind(f.w.namespace.id().as_uuid()).bind(member).bind(chain)
        .bind(expected).bind(epoch).bind(count).bind(possession).bind(holder)
        .bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(conn).await.map(|_| ())
}

#[tokio::test]
async fn active_holder_fk_is_deferred_and_abort_preserves_both_rows() {
    let f = fixture().await;
    let mut tx =
        f.w.execution_pool
            .begin()
            .await
            .expect("runtime transaction");
    finish_member(&mut tx, &f, f.member).await;
    error_is(
        tx.commit()
            .await
            .expect_err("active holder cannot finish alone"),
        "23503",
        Some("capacity_groups_holder_active_fk"),
    );
    assert_eq!(
        state(&f.w.db.pool, &f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    let flags: (bool,bool) = sqlx::query_as("SELECT condeferrable,condeferred FROM pg_constraint WHERE conrelid='capacity_groups'::regclass AND conname='capacity_groups_holder_active_fk'")
        .fetch_one(&f.w.db.pool).await.expect("constraint catalog");
    assert_eq!(flags, (true, true));
    let mut tx =
        f.w.execution_pool
            .begin()
            .await
            .expect("matching runtime transition");
    retire(&mut tx, &f).await;
    finish_member(&mut tx, &f, f.member).await;
    receipt(&mut tx, &f, f.member, f.chain, 1, (2, "RETIRED", None, 0))
        .await
        .expect("runtime receipt insert");
    tx.commit().await.expect("matching rows settle at commit");
    assert_eq!(
        state(&f.w.execution_pool, &f).await,
        (2, 2, 0, "RETIRED".into(), "FINISHED".into())
    );
}

#[tokio::test]
async fn runtime_has_only_selected_columns_and_receipt_append_privileges() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool().await;
    let privileges: (bool,bool,bool,bool,bool,bool) = sqlx::query_as("SELECT has_table_privilege(current_user,'capacity_groups','UPDATE'),has_column_privilege(current_user,'capacity_groups','revision','UPDATE'),has_column_privilege(current_user,'capacity_groups','tenant_id','UPDATE'),has_table_privilege(current_user,'capacity_group_members','UPDATE'),has_column_privilege(current_user,'capacity_group_members','state','UPDATE'),has_column_privilege(current_user,'capacity_group_members','chain_id','UPDATE')")
        .fetch_one(&runtime).await.expect("runtime privilege catalog");
    assert_eq!(privileges, (false, true, false, false, true, false));
    for statement in [
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) SELECT tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id FROM capacity_groups WHERE false",
        "INSERT INTO capacity_group_members SELECT * FROM capacity_group_members WHERE false",
        "DELETE FROM capacity_groups WHERE false",
        "TRUNCATE capacity_groups CASCADE",
        "DELETE FROM capacity_group_members WHERE false",
        "TRUNCATE capacity_group_members CASCADE",
        "UPDATE capacity_group_finish_receipts SET fingerprint=fingerprint WHERE false",
        "DELETE FROM capacity_group_finish_receipts WHERE false",
        "TRUNCATE capacity_group_finish_receipts",
        "UPDATE capacity_groups SET tenant_id=tenant_id WHERE false",
        "UPDATE capacity_group_members SET chain_id=chain_id WHERE false",
    ] {
        error_is(
            sqlx::query(statement)
                .execute(&runtime)
                .await
                .expect_err(statement),
            "42501",
            None,
        );
    }
    let policies: (i64,i64) = sqlx::query_as("SELECT count(*) FILTER(WHERE relrowsecurity), (SELECT count(*) FROM pg_policy WHERE polrelid IN ('capacity_groups'::regclass,'capacity_group_members'::regclass,'capacity_group_finish_receipts'::regclass)) FROM pg_class WHERE oid IN ('capacity_groups'::regclass,'capacity_group_members'::regclass,'capacity_group_finish_receipts'::regclass)")
        .fetch_one(&runtime).await.expect("explicit non-RLS baseline");
    assert_eq!(policies, (0, 0));
}

#[tokio::test]
async fn counters_identity_and_tenant_namespace_bindings_are_enforced() {
    let f = fixture().await;
    let pool = &f.w.db.pool;
    for (assignment, constraint) in [
        ("revision=0", "capacity_groups_counters"),
        ("epoch=2", "capacity_groups_counters"),
        ("active_members=4294967296", "capacity_groups_counters"),
        ("active_members=0", "capacity_groups_possession_consistent"),
        (
            "possession='OTHER'",
            "capacity_groups_possession_consistent",
        ),
    ] {
        let sql =
            format!("UPDATE capacity_groups SET {assignment} WHERE tenant_id=$1 AND group_id=$2");
        error_is(
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(f.w.admin.tenant_id().as_uuid())
                .bind(f.group)
                .execute(pool)
                .await
                .expect_err(assignment),
            "23514",
            Some(constraint),
        );
    }
    for (column, value) in [
        ("tenant_id", Uuid::new_v4()),
        ("namespace_id", Uuid::new_v4()),
        ("chain_id", Uuid::new_v4()),
        ("group_id", Uuid::new_v4()),
        ("member_id", Uuid::new_v4()),
    ] {
        // Only adjacent hard-coded test assignments enter this SQL.
        let sql = format!(
            "UPDATE capacity_group_members SET {column}=$1 WHERE tenant_id=$2 AND group_id=$3 AND member_id=$4"
        );
        error_is(
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(value)
                .bind(f.w.admin.tenant_id().as_uuid())
                .bind(f.group)
                .bind(f.member)
                .execute(pool)
                .await
                .expect_err(column),
            "23514",
            None,
        );
    }
    // Keep the group valid while breaking only the chain's exact namespace/tenant binding.
    let foreign = fixture().await;
    for (namespace, chain) in [
        (Uuid::new_v4(), f.chain),
        (f.w.namespace.id().as_uuid(), foreign.chain),
    ] {
        let error=sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
            .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(Uuid::new_v4()).bind(namespace).bind(chain)
            .execute(pool).await.expect_err("foreign namespace or tenant chain");
        error_is(error, "23503", Some("capacity_group_members_chain_fk"));
    }
    let mut conn = pool.acquire().await.expect("owner connection");
    let other = Uuid::new_v4();
    error_is(
        receipt(&mut conn, &f, other, f.chain, 1, (2, "RETIRED", None, 0))
            .await
            .expect_err("receipt cannot alias member"),
        "23503",
        Some("capacity_group_finish_receipts_member_binding_fk"),
    );
}

#[tokio::test]
async fn tombstones_and_receipts_survive_owner_mutation_attempts() {
    let f = fixture().await;
    let mut tx =
        f.w.execution_pool
            .begin()
            .await
            .expect("runtime transition");
    retire(&mut tx, &f).await;
    finish_member(&mut tx, &f, f.member).await;
    receipt(&mut tx, &f, f.member, f.chain, 1, (2, "RETIRED", None, 0))
        .await
        .expect("receipt");
    tx.commit().await.expect("retired fixture");
    for statement in [
        "UPDATE capacity_groups SET revision=revision+1",
        "UPDATE capacity_group_members SET state='ACTIVE',finished_at=NULL",
    ] {
        error_is(
            sqlx::query(statement)
                .execute(&f.w.db.pool)
                .await
                .expect_err(statement),
            "23514",
            None,
        );
    }
    for statement in [
        "DELETE FROM capacity_groups",
        "DELETE FROM capacity_group_members",
        "UPDATE capacity_group_finish_receipts SET fingerprint=fingerprint",
        "DELETE FROM capacity_group_finish_receipts",
        "TRUNCATE capacity_groups CASCADE",
        "TRUNCATE capacity_group_members CASCADE",
        "TRUNCATE capacity_group_finish_receipts",
    ] {
        error_is(
            sqlx::query(statement)
                .execute(&f.w.db.pool)
                .await
                .expect_err(statement),
            "23001",
            None,
        );
    }
    assert_eq!(
        state(&f.w.db.pool, &f).await,
        (2, 2, 0, "RETIRED".into(), "FINISHED".into())
    );
}

const BEFORE_CAPACITY: i64 = 20_261_209_000_000;
fn retained_downgrade(error: sqlx::migrate::MigrateError) {
    match error {
        sqlx::migrate::MigrateError::ExecuteMigration(error, version) => {
            assert_eq!(
                version, 20_261_210_000_000,
                "unexpected downgrade error: {error:?}"
            );
            error_is(error, "55000", None);
        }
        other => panic!("expected typed retained-storage downgrade refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn empty_schema_reverses_but_retained_tombstones_refuse_downgrade() {
    let w = world().await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE_CAPACITY)
        .await
        .expect("empty dormant schema downgrades");
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('capacity_groups') IS NOT NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("absence");
    assert!(!exists);
    migrate(&w.db.pool)
        .await
        .expect("dormant schema upgrades again");
    let f = fixture().await;
    let mut tx = f.w.db.pool.begin().await.expect("retired tombstone");
    retire(&mut tx, &f).await;
    finish_member(&mut tx, &f, f.member).await;
    receipt(&mut tx, &f, f.member, f.chain, 1, (2, "RETIRED", None, 0))
        .await
        .expect("receipt");
    tx.commit().await.expect("retained rows");
    retained_downgrade(
        MIGRATOR
            .undo(&f.w.db.pool, BEFORE_CAPACITY)
            .await
            .expect_err("retain compatible reader"),
    );
    assert_eq!(
        state(&f.w.db.pool, &f).await,
        (2, 2, 0, "RETIRED".into(), "FINISHED".into())
    );
    let version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("migration remains applied");
    assert_eq!(version, 20_261_210_000_000);
}

#[tokio::test]
async fn historical_holder_receipt_survives_later_holder_finish() {
    let f = fixture().await;
    let second_chain = seed_chain(&f.w, "capacity-second-member").await.as_uuid();
    let second = Uuid::new_v4();
    let mut tx = f.w.db.pool.begin().await.expect("second fixture member");
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=2 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .execute(&mut *tx)
    .await
    .expect("fixture join header");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(second).bind(f.w.namespace.id().as_uuid()).bind(second_chain)
        .execute(&mut *tx).await.expect("fixture join binding");
    tx.commit().await.expect("two member fixture");
    let mut tx =
        f.w.execution_pool
            .begin()
            .await
            .expect("nonholder transition");
    finish_member(&mut tx, &f, second).await;
    sqlx::query(
        "UPDATE capacity_groups SET revision=3,active_members=1 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .execute(&mut *tx)
    .await
    .expect("nonholder leaves holder alone");
    receipt(
        &mut tx,
        &f,
        second,
        second_chain,
        2,
        (1, "HELD", Some(f.member), 1),
    )
    .await
    .expect("historical holder identity");
    tx.commit().await.expect("nonholder schema transition");
    let mut tx = f.w.execution_pool.begin().await.expect("holder transition");
    retire(&mut tx, &f).await;
    finish_member(&mut tx, &f, f.member).await;
    receipt(&mut tx, &f, f.member, f.chain, 3, (2, "RETIRED", None, 0))
        .await
        .expect("last member receipt");
    tx.commit()
        .await
        .expect("historical receipt does not require holder ACTIVE forever");
    let result: (i64,Uuid,i64) = sqlx::query_as("SELECT resulting_epoch,resulting_holder_member_id,(SELECT count(*) FROM capacity_group_finish_receipts) FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(second).fetch_one(&f.w.execution_pool).await.expect("original receipt unchanged");
    assert_eq!(result, (1, f.member, 2));
    let mut conn = f.w.db.pool.acquire().await.expect("owner connection");
    // Use a new operation key and a new resulting revision: only the member alias is duplicated.
    error_is(
        receipt(&mut conn, &f, f.member, f.chain, 4, (2, "RETIRED", None, 0))
            .await
            .expect_err("one finish per member"),
        "23505",
        Some("capacity_group_finish_receipts_member_unique"),
    );
}

#[tokio::test]
async fn vacant_nonholder_receipt_keeps_epoch_and_fixed_fingerprint_bounds() {
    let f = fixture().await;
    let other_chain = seed_chain(&f.w, "vacant-other-member").await.as_uuid();
    let mut tx = f.w.db.pool.begin().await.expect("vacant fixture");
    sqlx::query("UPDATE capacity_groups SET revision=3,epoch=2,active_members=2,possession='VACANT',holder_member_id=NULL WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&mut *tx).await.expect("fixture vacant baseline");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(Uuid::new_v4()).bind(f.w.namespace.id().as_uuid()).bind(other_chain)
        .execute(&mut *tx).await.expect("other active vacant member");
    finish_member(&mut tx, &f, f.member).await;
    // This is a journal schema probe, not a fabricated full aggregate/production history.
    sqlx::query("INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,finished_at,audit_id,outbox_id) VALUES($1,$2,$3,$4,$5,$6,$7,$7,3,2,0,0,4,2,1,'VACANT',clock_timestamp(),$8,$9)")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(Uuid::new_v4()).bind(vec![0_u8;32]).bind(f.w.namespace.id().as_uuid()).bind(f.member).bind(f.chain).bind(Uuid::new_v4()).bind(Uuid::new_v4())
        .execute(&mut *tx).await.expect("vacant nonholder does not increment epoch");
    sqlx::query(
        "UPDATE capacity_groups SET revision=4,active_members=1 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .execute(&mut *tx)
    .await
    .expect("matching vacant nonholder header");
    tx.commit().await.expect("valid vacant receipt");
    // Copy the otherwise valid row into a statement whose fingerprint check is invalid.
    for length in [0, 31, 33] {
        let error=sqlx::query("INSERT INTO capacity_group_finish_receipts SELECT tenant_id,group_id,$1,$2,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id FROM capacity_group_finish_receipts")
            .bind(Uuid::new_v4()).bind(vec![0_u8;length]).execute(&f.w.db.pool).await.expect_err("fingerprint bound");
        error_is(
            error,
            "23514",
            Some("capacity_group_finish_receipts_fingerprint"),
        );
    }
}

#[tokio::test]
async fn downgrade_waits_for_writer_before_deciding_whether_storage_is_empty() {
    let w = world().await;
    // Transfer-receipt rollback is explicitly quiesced-only. Remove that later schema
    // before starting the concurrent writer that probes Finish's own rollback guard.
    MIGRATOR
        .undo(&w.db.pool, 20_261_225_000_000)
        .await
        .expect("quiesced transfer rollback before the Finish writer race");
    let mut writer =
        w.db.pool
            .begin()
            .await
            .expect("uncommitted dormant creator fixture");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .expect("writer pid");
    sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession) VALUES($1,$2,1,1,0,'RETIRED')")
        .bind(w.admin.tenant_id().as_uuid()).bind(Uuid::new_v4()).execute(&mut *writer).await.expect("uncommitted tombstone fixture");
    let pool = w.db.pool.clone();
    let downgrade = tokio::spawn(async move { MIGRATOR.undo(&pool, BEFORE_CAPACITY).await });
    // Observe the exact relation/mode/blocker, not the spelling of the current DDL.
    crate::support::eventually("downgrade actually waits on writer before emptiness check",|| async {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND relation='capacity_groups'::regclass AND mode='AccessExclusiveLock' AND NOT granted AND $1=ANY(pg_blocking_pids(pid)))")
            .bind(pid).fetch_one(&w.db.pool).await.expect("actual migration lock wait").then_some(())
    }).await;
    writer.commit().await.expect("writer commits first");
    retained_downgrade(
        downgrade
            .await
            .expect("migration task")
            .expect_err("new committed tombstone prevents downgrade"),
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM capacity_groups")
        .fetch_one(&w.db.pool)
        .await
        .expect("committed tombstone survives");
    assert_eq!(count, 1);
}

// Native-operation fixtures below use real child creation and worker completion. Only the dormant
// capacity membership is owner-seeded; finality/continuation is never fabricated with UPDATEs.
struct NativeFixture {
    f: Fixture,
    parent: RunId,
    scheduler: crate::support::engine::Sched,
}

async fn spawn_native_child(w: &World, continue_first: bool) -> (RunId, RunId) {
    use crate::support::engine::{activity, child_node, document};
    use sagat_application::execution::child_run_id;
    use sagat_domain::workflow::NodeId;
    let child_name = format!("capacity-child-{}", Uuid::new_v4());
    w.publish(serde_json::json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
        "metadata":{"name":child_name}, "entrypoint":"work",
        "nodes":[
            {"id":"work","type":"activity","activity":"capacity.work","next":"again"},
            {"id":"again","type":"condition","expression":"input.continue","ifTrue":"rollover","ifFalse":"done"},
            {"id":"rollover","type":"continueAsNew","input":"input.next"},
            {"id":"done","type":"end"}
        ], "chainPolicy":{"maxSegments":3,"timeoutMs":3600000}
    })).await;
    let (workflow, _) = w
        .publish(document(
            &format!("capacity-parent-{}", Uuid::new_v4()),
            "kid",
            vec![
                child_node("kid", &child_name, Some("after")),
                activity("after"),
            ],
        ))
        .await;
    let parent = w
        .start(
            workflow,
            &serde_json::json!({"continue":continue_first,"next":{"continue":false}}),
        )
        .await;
    let child = child_run_id(parent, &NodeId::parse("kid").expect("node"));
    (parent, child)
}

async fn native_fixture(continue_first: bool) -> NativeFixture {
    let w = world().await;
    let (parent, child) = spawn_native_child(&w, continue_first).await;
    let scheduler = super::chains::scheduler(&w, "capacity-native").await;
    let report = scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    let f = seed_capacity(w, child).await;
    NativeFixture {
        f,
        parent,
        scheduler,
    }
}

async fn complete_native_activity(n: &NativeFixture) -> RunId {
    let session = n.f.w.session(&["capacity.work"]).await;
    // Request one task: the general one_task_of helper leases up to eight and would
    // discard another ready peer, leaving that peer leased to a different test session.
    let mut tasks = n.f.w.poll(&session, 1).await;
    assert_eq!(tasks.len(), 1, "one current child activity");
    let task = tasks.remove(0);
    assert_eq!(task.activity.as_str(), "capacity.work");
    n.f.w
        .complete(&session, &task, &serde_json::json!({}))
        .await
        .expect("real child activity completes");
    task.run_id
}

#[tokio::test]
async fn native_fixture_reaches_current_child_final_through_real_runtime_writers() {
    let n = native_fixture(false).await;
    let run = complete_native_activity(&n).await;
    assert_eq!(run.as_uuid(), n.f.chain);
    let finality: (Uuid,String,String,bool,bool,Option<Uuid>)=sqlx::query_as("SELECT c.current_run_id,c.final_outcome,s.completion_kind,r.finished_at IS NOT NULL,s.completed_at IS NOT NULL,s.successor_run_id FROM execution_chains c JOIN chain_segments s USING(tenant_id,namespace_id,chain_id) JOIN runs r ON r.tenant_id=s.tenant_id AND r.id=s.run_id WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.chain_id=$3 AND s.run_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).fetch_one(&n.f.w.execution_pool).await.expect("real child final projection");
    assert_eq!(
        finality,
        (
            n.f.chain,
            "SUCCEEDED".into(),
            "FINAL".into(),
            true,
            true,
            None
        )
    );
    let parent:Uuid=sqlx::query_scalar("SELECT parent_run_id FROM child_chain_links WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).fetch_one(&n.f.w.execution_pool).await.expect("actual child association");
    assert_eq!(parent, n.parent.as_uuid());
    assert_eq!(n.scheduler.tick().await.failed, 0);
}

use sagat_application::capacity_finish::{
    CapacityFinishCommand, CapacityFinishExpectations, CapacityFinishIdentity,
    CurrentChildRootFinishStore,
};
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::execution::{CapacityGroupId, CapacityMemberId, CapacityPossession, ChainScope};
use sagat_infra_postgres::PgCapacityFinishStore;

async fn native_command(n: &NativeFixture, head: RunId) -> CapacityFinishCommand {
    native_member_command(&n.f, n.f.member, RunId::new(n.f.chain), head).await
}

async fn native_member_command(
    f: &Fixture,
    member: Uuid,
    chain: RunId,
    head: RunId,
) -> CapacityFinishCommand {
    let (chain_revision,run_revision): (i64,i64)=sqlx::query_as("SELECT c.revision,r.run_version FROM execution_chains c JOIN runs r ON r.tenant_id=c.tenant_id AND r.id=$4 WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.chain_id=$3")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.w.namespace.id().as_uuid()).bind(chain.as_uuid()).bind(head.as_uuid()).fetch_one(&f.w.db.pool).await.expect("observed exact command revisions");
    let (group_revision, group_epoch): (i64, i64) = sqlx::query_as(
        "SELECT revision,epoch FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("observed group counters");
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(head.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("current scheduler lease");
    CapacityFinishCommand::new(
        Uuid::new_v4(),
        CapacityFinishIdentity::new(
            ChainScope::new(f.w.admin.tenant_id(), f.w.namespace.id()),
            CapacityGroupId::new(f.group),
            CapacityMemberId::new(member),
            chain,
            head,
        ),
        CapacityFinishExpectations::new(
            u64::try_from(group_revision).expect("group revision"),
            u64::try_from(group_epoch).expect("group epoch"),
            u64::try_from(chain_revision).expect("chain revision"),
            u64::try_from(run_revision).expect("run revision"),
        ),
        SchedulerAuthority::for_run(owner, head, u64::try_from(generation).expect("generation")),
    )
}

async fn execution_snapshot(f: &Fixture) -> Vec<(String, String)> {
    sqlx::query_as("SELECT 'runs',md5(coalesce(jsonb_agg(to_jsonb(r) ORDER BY id)::text,'null')) FROM runs r WHERE tenant_id=$1 UNION ALL SELECT 'events',md5(coalesce(jsonb_agg(to_jsonb(e) ORDER BY run_id,seq)::text,'null')) FROM run_events e WHERE tenant_id=$1 UNION ALL SELECT 'steps',md5(coalesce(jsonb_agg(to_jsonb(s) ORDER BY run_id,node)::text,'null')) FROM steps s WHERE tenant_id=$1 UNION ALL SELECT 'chains',md5(coalesce(jsonb_agg(to_jsonb(c) ORDER BY chain_id)::text,'null')) FROM execution_chains c WHERE tenant_id=$1 UNION ALL SELECT 'segments',md5(coalesce(jsonb_agg(to_jsonb(s) ORDER BY chain_id,segment_index)::text,'null')) FROM chain_segments s WHERE tenant_id=$1 UNION ALL SELECT 'telemetry',md5(coalesce(jsonb_agg(to_jsonb(t) ORDER BY run_id)::text,'null')) FROM telemetry_pending t WHERE tenant_id=$1 UNION ALL SELECT 'usage',md5(coalesce(jsonb_agg(to_jsonb(u))::text,'null')) FROM tenant_retained_usage u WHERE tenant_id=$1")
        .bind(f.w.admin.tenant_id().as_uuid()).fetch_all(&f.w.db.pool).await.expect("independent immutable execution snapshot")
}

async fn finish_counts(f: &Fixture) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='capacity.member.finished' AND resource_id=$3),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='capacity.member.finished')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(f.group.to_string()).fetch_one(&f.w.db.pool).await.expect("capacity side effect counts")
}

#[tokio::test]
async fn current_child_final_finishes_once_without_rewriting_execution() {
    let n = native_fixture(false).await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let before = execution_snapshot(&n.f).await;
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    let applied = store
        .finish_current_child_root(&command)
        .await
        .expect("current real child FINAL");
    assert!(!applied.replayed);
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession
        ),
        (2, 2, 0, CapacityPossession::Retired)
    );
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (2, 2, 0, "RETIRED".into(), "FINISHED".into())
    );
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    let mut replay = store
        .finish_current_child_root(&command)
        .await
        .expect("same command replays even after retirement");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(replay, applied, "original immutable receipt result");
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(execution_snapshot(&n.f).await, before);
    super::hint_contract::assert_outbox_matches_the_contract(&n.f.w.db.pool).await;
}

use sagat_application::capacity_finish::CapacityFinishError;

#[tokio::test]
async fn native_finish_rejects_live_wrong_scope_stale_fence_and_conflicting_replay() {
    let n = native_fixture(false).await;
    let live = native_command(&n, RunId::new(n.f.chain)).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        store.finish_current_child_root(&live).await,
        Err(CapacityFinishError::NotFinal)
    );
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let before = execution_snapshot(&n.f).await;
    let mut variants = Vec::new();
    let mut c = command.clone();
    c.identity.scope = ChainScope::new(n.f.w.other_admin.tenant_id(), n.f.w.other_namespace.id());
    variants.push((c, CapacityFinishError::Missing));
    let mut c = command.clone();
    c.identity.scope = ChainScope::new(n.f.w.admin.tenant_id(), n.f.w.other_namespace.id());
    variants.push((c, CapacityFinishError::BindingMismatch));
    let mut c = command.clone();
    c.identity.member_id = CapacityMemberId::generate();
    variants.push((c, CapacityFinishError::Missing));
    let mut c = command.clone();
    c.identity.group_id = CapacityGroupId::generate();
    variants.push((c, CapacityFinishError::Missing));
    let mut c = command.clone();
    c.identity.chain_id = RunId::generate();
    variants.push((c, CapacityFinishError::BindingMismatch));
    let mut c = command.clone();
    c.identity.head_run_id = n.parent;
    variants.push((c, CapacityFinishError::StaleHead { current: head }));
    let mut c = command.clone();
    c.scheduler = SchedulerAuthority::for_run("wrong-owner", head, 1);
    variants.push((c, CapacityFinishError::SchedulerFenceLost));
    let mut c = command.clone();
    c.expected.group_revision += 1;
    variants.push((c, CapacityFinishError::StaleGroupRevision { current: 1 }));
    let mut c = command.clone();
    c.expected.group_epoch += 1;
    variants.push((c, CapacityFinishError::StaleGroupEpoch { current: 1 }));
    let mut c = command.clone();
    c.expected.chain_revision += 1;
    variants.push((
        c,
        CapacityFinishError::StaleChainRevision {
            current: command.expected.chain_revision,
        },
    ));
    let mut c = command.clone();
    c.expected.run_revision += 1;
    variants.push((
        c,
        CapacityFinishError::StaleRunRevision {
            current: command.expected.run_revision,
        },
    ));
    let mut c = command.clone();
    c.expected.run_revision = u64::MAX;
    variants.push((c, CapacityFinishError::CounterExhausted));
    for (c, expected) in variants {
        assert_eq!(store.finish_current_child_root(&c).await, Err(expected));
        assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
        assert_eq!(
            state(&n.f.w.db.pool, &n.f).await,
            (1, 1, 1, "HELD".into(), "ACTIVE".into())
        );
    }
    let applied = store
        .finish_current_child_root(&command)
        .await
        .expect("fresh valid command");
    let mut conflict = command.clone();
    conflict.expected.group_revision += 1;
    assert_eq!(
        store.finish_current_child_root(&conflict).await,
        Err(CapacityFinishError::FingerprintConflict)
    );
    let generation:i64=sqlx::query_scalar("UPDATE scheduler_shards SET generation=generation+1,owner='capacity-replay-renewed',expires_at=clock_timestamp()+interval '1 hour' WHERE shard=run_shard($1) RETURNING generation")
        .bind(head.as_uuid()).fetch_one(&n.f.w.db.pool).await.expect("renew scheduler authority");
    assert_eq!(
        store.finish_current_child_root(&command).await,
        Err(CapacityFinishError::SchedulerFenceLost),
        "replay is fenced too"
    );
    let mut renewed = command.clone();
    renewed.scheduler = SchedulerAuthority::for_run(
        "capacity-replay-renewed",
        head,
        u64::try_from(generation).expect("generation"),
    );
    assert_eq!(renewed.fingerprint(), command.fingerprint());
    let mut replay = store
        .finish_current_child_root(&renewed)
        .await
        .expect("renewed authority replays same logical command");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(replay, applied);
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(execution_snapshot(&n.f).await, before);
}

#[tokio::test]
async fn continued_history_never_finishes_logical_member_even_after_successor_final() {
    let n = native_fixture(true).await;
    let initial = complete_native_activity(&n).await;
    let old = native_command(&n, initial).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        store.finish_current_child_root(&old).await,
        Err(CapacityFinishError::NotFinal)
    );
    let report = n.scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0);
    let successor:Uuid=sqlx::query_scalar("SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).fetch_one(&n.f.w.db.pool).await.expect("new head");
    let successor = RunId::new(successor);
    assert_ne!(initial, successor);
    assert_eq!(
        store.finish_current_child_root(&old).await,
        Err(CapacityFinishError::StaleHead { current: successor })
    );
    let last = complete_native_activity(&n).await;
    assert_eq!(last, successor);
    assert_eq!(
        store.finish_current_child_root(&old).await,
        Err(CapacityFinishError::StaleHead { current: successor })
    );
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    let before = execution_snapshot(&n.f).await;
    store
        .finish_current_child_root(&native_command(&n, successor).await)
        .await
        .expect("current successor FINAL settles retained logical membership");
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(execution_snapshot(&n.f).await, before);
}

#[tokio::test]
async fn exhausted_sql_revision_leaves_no_partial_finish() {
    let n = native_fixture(false).await;
    let head = complete_native_activity(&n).await;
    sqlx::query("UPDATE capacity_groups SET revision=$3 WHERE tenant_id=$1 AND group_id=$2")
        .bind(n.f.w.admin.tenant_id().as_uuid())
        .bind(n.f.group)
        .bind(i64::MAX)
        .execute(&n.f.w.db.pool)
        .await
        .expect("maximum persisted revision fixture");
    let mut command = native_command(&n, head).await;
    command.expected.group_revision = u64::try_from(i64::MAX).expect("positive maximum");
    let before = execution_snapshot(&n.f).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        store.finish_current_child_root(&command).await,
        Err(CapacityFinishError::CounterExhausted)
    );
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (i64::MAX, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    assert_eq!(execution_snapshot(&n.f).await, before);
}

#[tokio::test]
async fn finishing_selected_child_preserves_other_members_and_historical_receipt() {
    for selected_holds in [true, false] {
        let n = native_fixture(false).await;
        let selected_head = complete_native_activity(&n).await;
        let (_, a) = spawn_native_child(&n.f.w, false).await;
        let (_, c) = spawn_native_child(&n.f.w, false).await;
        let report = n.scheduler.tick().await;
        assert_eq!(report.children_started, 2, "{report:?}");
        assert_eq!(report.failed, 0);
        let a_member = Uuid::new_v4();
        let c_member = Uuid::new_v4();
        let mut tx =
            n.f.w
                .db
                .pool
                .begin()
                .await
                .expect("three-member owner fixture");
        for (member, chain) in [(a_member, a), (c_member, c)] {
            sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(member).bind(n.f.w.namespace.id().as_uuid()).bind(chain.as_uuid()).execute(&mut *tx).await.expect("real child member fixture");
        }
        sqlx::query("UPDATE capacity_groups SET revision=3,active_members=3,holder_member_id=$3 WHERE tenant_id=$1 AND group_id=$2")
            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(if selected_holds{n.f.member}else{c_member}).execute(&mut *tx).await.expect("fixture header");
        tx.commit().await.expect("bounded group baseline");
        let before = execution_snapshot(&n.f).await;
        let command = native_command(&n, selected_head).await;
        let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
        let original = store
            .finish_current_child_root(&command)
            .await
            .expect("selected FINAL only");
        let expected = if selected_holds {
            CapacityPossession::Vacant
        } else {
            CapacityPossession::Held(CapacityMemberId::new(c_member))
        };
        assert_eq!(
            (
                original.revision,
                original.epoch,
                original.active_members,
                original.possession
            ),
            (4, if selected_holds { 2 } else { 1 }, 2, expected)
        );
        let survivors:Vec<(Uuid,String)>=sqlx::query_as("SELECT member_id,state FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 AND member_id<>$3 ORDER BY member_id")
            .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(n.f.member).fetch_all(&n.f.w.db.pool).await.expect("independent survivors");
        assert_eq!(survivors.len(), 2);
        assert!(survivors.iter().all(|(_, state)| state == "ACTIVE"));
        assert_eq!(
            execution_snapshot(&n.f).await,
            before,
            "no other run/chain business state changes"
        );
        for _ in 0..2 {
            let head = complete_native_activity(&n).await;
            let member = if head == a {
                a_member
            } else {
                assert_eq!(head, c);
                c_member
            };
            let command = native_member_command(&n.f, member, head, head).await;
            store
                .finish_current_child_root(&command)
                .await
                .expect("each peer's own current FINAL");
        }
        let final_state = state(&n.f.w.db.pool, &n.f).await;
        assert_eq!(final_state.0, 6);
        assert_eq!(final_state.2, 0);
        assert_eq!(final_state.3, "RETIRED");
        assert_eq!(finish_counts(&n.f).await, (3, 3, 3));
        let mut replay = store
            .finish_current_child_root(&command)
            .await
            .expect("original historical result after every survivor finishes");
        assert!(replay.replayed);
        replay.replayed = false;
        assert_eq!(replay, original);
        assert_eq!(finish_counts(&n.f).await, (3, 3, 3));
    }
}

#[tokio::test]
async fn contradictory_final_outcome_refuses_finish_and_root_binding_grants_no_authority() {
    let n = native_fixture(false).await;
    let head = complete_native_activity(&n).await;
    // Explicit persisted-fault injection after genuine runtime FINAL, not a finality fixture.
    sqlx::query("UPDATE execution_chains SET final_outcome='FAILED' WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).execute(&n.f.w.db.pool).await.expect("contradictory outcome fault");
    let before = execution_snapshot(&n.f).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        store
            .finish_current_child_root(&native_command(&n, head).await)
            .await,
        Err(CapacityFinishError::ContradictoryProjection)
    );
    assert_eq!(execution_snapshot(&n.f).await, before);
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));

    let f = fixture().await;
    let _scheduler = super::chains::scheduler(&f.w, "capacity-root-refusal").await;
    let before = execution_snapshot(&f).await;
    let command =
        native_member_command(&f, f.member, RunId::new(f.chain), RunId::new(f.chain)).await;
    let store = PgCapacityFinishStore::new(f.w.execution_pool.clone());
    assert_eq!(
        store.finish_current_child_root(&command).await,
        Err(CapacityFinishError::BindingMismatch),
        "CHILD_ROOT text in a registry row never proves physical child lineage"
    );
    assert_eq!(execution_snapshot(&f).await, before);
    assert_eq!(finish_counts(&f).await, (0, 0, 0));
}

#[path = "capacity_finish_barriers.rs"]
mod barriers;

#[path = "capacity_finish_commit_loss.rs"]
mod commit_loss;
#[path = "capacity_finish_proxy.rs"]
pub(crate) mod commit_proxy;

#[path = "capacity_finish_races.rs"]
mod races;

#[path = "capacity_finish_observations.rs"]
mod observations;

#[path = "capacity_scoped_binding.rs"]
mod scoped_binding;

#[path = "capacity_discovery.rs"]
mod discovery;

#[path = "capacity_vacate.rs"]
mod vacate;

//! Schema-only collection run-data-limit evidence.
//!
//! The public empty collection supplies a genuine Run and namespace. Direct scope/event writes in
//! this module are explicitly static storage fixtures: they do not claim a native quota producer,
//! a certified collection source, replay verification, or legacy mixed-writer compatibility.

use crate::support::blob_collections::{collection_ir, platform, start_inline};
use crate::support::worker::Platform;
use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::json;
use sqlx::{PgPool, migrate::MigrateError};
use uuid::Uuid;

const BEFORE_RUN_DATA_LIMIT: i64 = 20_261_212_000_000;
const RUN_DATA_LIMIT_VERSION: i64 = 20_261_213_000_000;

/// The newest migration: later ones (feature 016's role-change trigger) sit above the run-data-limit
/// vocabulary this module exercises, so a fully migrated database reports the head, not that one.
fn head_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .expect("migrations")
}
struct SchemaFixture {
    platform: Platform,
    run: Uuid,
    scope: Uuid,
}

#[derive(Clone)]
struct Projection {
    lifecycle: &'static str,
    drain_kind: Option<&'static str>,
    drain_iteration: Option<i32>,
    drain_failure_code: Option<&'static str>,
    drain_sequence: Option<i64>,
    drain_outbox_id: Option<Uuid>,
    finish_sequence: Option<i64>,
    finish_outbox_id: Option<Uuid>,
}

impl Projection {
    fn old_open(lifecycle: &'static str) -> Self {
        let terminal = lifecycle == "SUCCEEDED";
        Self {
            lifecycle,
            drain_kind: None,
            drain_iteration: None,
            drain_failure_code: None,
            drain_sequence: None,
            drain_outbox_id: None,
            finish_sequence: terminal.then_some(2),
            finish_outbox_id: terminal.then(Uuid::new_v4),
        }
    }

    fn drain(
        lifecycle: &'static str,
        kind: &'static str,
        iteration: Option<i32>,
        code: Option<&'static str>,
        sequence: i64,
    ) -> Self {
        let terminal = matches!(lifecycle, "FAILED" | "CANCELLED");
        Self {
            lifecycle,
            drain_kind: Some(kind),
            drain_iteration: iteration,
            drain_failure_code: code,
            drain_sequence: Some(sequence),
            drain_outbox_id: Some(Uuid::new_v4()),
            finish_sequence: terminal.then_some(sequence + 1),
            finish_outbox_id: terminal.then(Uuid::new_v4),
        }
    }
}

async fn fixture() -> SchemaFixture {
    let platform = platform().await;
    let name = "collection-data-limit-schema";
    let workflow = platform
        .publish(name, &collection_ir(name, "input.items", 1))
        .await;
    let run = start_inline(&platform, &workflow, &json!({"items": []})).await;
    let scope = Uuid::new_v4();
    let tenant = platform.tenant.as_uuid();
    let (namespace, snapshot): (Uuid, Vec<u8>) =
        sqlx::query_as("SELECT namespace_id,input_digest FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(run)
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("public empty collection Run");

    // Static projection fixture only. No runtime path currently produces RUN_DATA_LIMIT.
    sqlx::query(
        "INSERT INTO iteration_scopes(tenant_id,run_id,scope_id,namespace_id, \
         owner_path_nodes,owner_path_indices,owner_node,kind,lifecycle,snapshot_digest, \
         item_count,max_items,max_concurrency,created_at,updated_at) \
         VALUES($1,$2,$3,$4,'{}','{}','items','COLLECTION','OPEN',$5,0,1,1, \
         clock_timestamp(),clock_timestamp())",
    )
    .bind(tenant)
    .bind(run)
    .bind(scope)
    .bind(namespace)
    .bind(snapshot)
    .execute(&platform.api.db.pool)
    .await
    .expect("explicit static OPEN collection projection");

    SchemaFixture {
        platform,
        run,
        scope,
    }
}

async fn assert_candidate(fixture: &SchemaFixture, candidate: Projection, accepted: bool) {
    let mut transaction = fixture
        .platform
        .api
        .db
        .pool
        .begin()
        .await
        .expect("candidate transaction");
    sqlx::query("SAVEPOINT collection_data_limit_candidate")
        .execute(&mut *transaction)
        .await
        .expect("candidate savepoint");
    let result = sqlx::query(
        "UPDATE iteration_scopes SET lifecycle=$4,drain_kind=$5,drain_iteration=$6, \
         drain_failure_code=$7,drain_sequence=$8,drain_outbox_id=$9, \
         result_digest=CASE WHEN $4='SUCCEEDED' THEN snapshot_digest ELSE NULL END, \
         finish_sequence=$10,finish_outbox_id=$11,updated_at=clock_timestamp() \
         WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
    )
    .bind(fixture.platform.tenant.as_uuid())
    .bind(fixture.run)
    .bind(fixture.scope)
    .bind(candidate.lifecycle)
    .bind(candidate.drain_kind)
    .bind(candidate.drain_iteration)
    .bind(candidate.drain_failure_code)
    .bind(candidate.drain_sequence)
    .bind(candidate.drain_outbox_id)
    .bind(candidate.finish_sequence)
    .bind(candidate.finish_outbox_id)
    .execute(&mut *transaction)
    .await;
    if accepted {
        assert_eq!(
            result
                .expect("semantically valid drain projection")
                .rows_affected(),
            1
        );
    } else {
        let (code, constraint) =
            crate::support::db_error(result.expect_err("invalid drain projection is refused"));
        assert_eq!(code, "23514");
        assert_eq!(constraint.as_deref(), Some("iteration_scopes_drain_reason"));
    }
    sqlx::query("ROLLBACK TO SAVEPOINT collection_data_limit_candidate")
        .execute(&mut *transaction)
        .await
        .expect("candidate rollback");
    transaction
        .rollback()
        .await
        .expect("discard static candidate");
}

fn retained_reason(error: MigrateError) {
    match error {
        MigrateError::ExecuteMigration(error, version) => {
            assert_eq!(version, RUN_DATA_LIMIT_VERSION);
            assert_eq!(crate::support::db_error(error).0, "55000");
        }
        other => panic!("expected typed retained-reason downgrade refusal, got {other:?}"),
    }
}

async fn migration_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
        .expect("migration ledger")
}

async fn constraint_definition(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conrelid='iteration_scopes'::regclass \
           AND conname='iteration_scopes_drain_reason'",
    )
    .fetch_one(pool)
    .await
    .expect("collection drain constraint")
}

async fn observe_down_before_writer_queue(pool: &PgPool, pids: [i32; 3]) {
    assert!(
        pids[0] != pids[1] && pids[0] != pids[2] && pids[1] != pids[2],
        "gate, downgrade and writer use three distinct sessions",
    );
    crate::support::eventually("exact down-before-writer three-session queue", || async {
        sqlx::query_scalar::<_, bool>(
            r#"SELECT
              (SELECT count(*) FROM pg_stat_activity
                WHERE pid=ANY($1) AND datname=current_database())=3
              AND EXISTS(
                SELECT 1 FROM pg_stat_activity gate
                WHERE gate.pid=$2 AND gate.datname=current_database()
                  AND gate.state='idle in transaction'
                  AND position('LOCK TABLE iteration_scopes IN ACCESS EXCLUSIVE MODE'
                      in gate.query)>0)
              AND EXISTS(
                SELECT 1 FROM pg_stat_activity down
                WHERE down.pid=$3 AND down.datname=current_database()
                  AND down.wait_event_type='Lock'
                  AND position('LOCK TABLE iteration_scopes IN ACCESS EXCLUSIVE MODE'
                      in down.query)>0
                  AND $2=ANY(pg_blocking_pids(down.pid)))
              AND EXISTS(
                SELECT 1 FROM pg_stat_activity writer
                WHERE writer.pid=$4 AND writer.datname=current_database()
                  AND writer.wait_event_type='Lock'
                  AND position('SELECT run_version + 1 FROM runs' in writer.query)>0
                  AND $3=ANY(pg_blocking_pids(writer.pid)))
              AND EXISTS(
                SELECT 1 FROM pg_locks
                WHERE pid=$2 AND relation='iteration_scopes'::regclass
                  AND mode='AccessExclusiveLock' AND granted)
              AND EXISTS(
                SELECT 1 FROM pg_locks
                WHERE pid=$3 AND relation='runs'::regclass
                  AND mode='AccessExclusiveLock' AND granted)
              AND EXISTS(
                SELECT 1 FROM pg_locks
                WHERE pid=$3 AND relation='iteration_scopes'::regclass
                  AND mode='AccessExclusiveLock' AND NOT granted)
              AND EXISTS(
                SELECT 1 FROM pg_locks
                WHERE pid=$4 AND relation='runs'::regclass
                  AND mode='RowShareLock' AND NOT granted)"#,
        )
        .bind(pids.to_vec())
        .bind(pids[0])
        .bind(pids[1])
        .bind(pids[2])
        .fetch_one(pool)
        .await
        .expect("three-session pg_stat_activity/pg_locks witness")
        .then_some(())
    })
    .await;
}

#[tokio::test]
async fn run_data_limit_constraint_is_total_and_preserves_every_old_form() {
    let fixture = fixture().await;
    let valid = [
        Projection::old_open("OPEN"),
        Projection::old_open("SUCCEEDED"),
        Projection::drain("DRAINING", "FAILED", Some(0), Some("WORK_FAILED"), 1),
        Projection::drain("FAILED", "FAILED", Some(0), Some("WORK_FAILED"), 1),
        Projection::drain(
            "DRAINING",
            "STORAGE_QUOTA",
            None,
            Some("STORAGE_QUOTA_EXCEEDED"),
            1,
        ),
        Projection::drain(
            "FAILED",
            "STORAGE_QUOTA",
            None,
            Some("STORAGE_QUOTA_EXCEEDED"),
            1,
        ),
        Projection::drain("DRAINING", "CANCELLED", None, None, 1),
        Projection::drain("CANCELLED", "CANCELLED", None, None, 1),
        Projection::drain(
            "DRAINING",
            "RUN_DATA_LIMIT",
            None,
            Some("RUN_DATA_LIMIT"),
            1,
        ),
        Projection::drain("FAILED", "RUN_DATA_LIMIT", None, Some("RUN_DATA_LIMIT"), 1),
    ];
    for candidate in valid {
        assert_candidate(&fixture, candidate, true).await;
    }

    let missing_reason = Projection::old_open("DRAINING");
    let mut null_kind = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        1,
    );
    null_kind.drain_kind = None;
    let mut null_run_data_limit_code = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        1,
    );
    null_run_data_limit_code.drain_failure_code = None;
    let mut null_sequence = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        1,
    );
    null_sequence.drain_sequence = None;
    let mut null_failed_code =
        Projection::drain("DRAINING", "FAILED", Some(0), Some("WORK_FAILED"), 1);
    null_failed_code.drain_failure_code = None;
    let mut null_storage_quota_code = Projection::drain(
        "DRAINING",
        "STORAGE_QUOTA",
        None,
        Some("STORAGE_QUOTA_EXCEEDED"),
        1,
    );
    null_storage_quota_code.drain_failure_code = None;
    let wrong_code = Projection::drain("DRAINING", "RUN_DATA_LIMIT", None, Some("OTHER"), 1);
    let unexpected_iteration = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        Some(0),
        Some("RUN_DATA_LIMIT"),
        1,
    );
    let zero_sequence = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        0,
    );
    let mut missing_outbox = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        1,
    );
    missing_outbox.drain_outbox_id = None;
    for candidate in [
        missing_reason,
        null_kind,
        null_run_data_limit_code,
        null_sequence,
        null_failed_code,
        null_storage_quota_code,
        wrong_code,
        unexpected_iteration,
        zero_sequence,
        missing_outbox,
    ] {
        assert_candidate(&fixture, candidate, false).await;
    }
}

#[tokio::test]
async fn ordinary_history_without_the_new_reason_downgrades_and_reapplies() {
    let fixture = fixture().await;
    let pool = &fixture.platform.api.db.pool;
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM runs WHERE tenant_id=$1 AND id=$2), \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2)",
    )
    .bind(fixture.platform.tenant.as_uuid())
    .bind(fixture.run)
    .fetch_one(pool)
    .await
    .expect("ordinary public history");
    assert!(before.0 > 0);
    assert!(before.1 > 0);

    let mut holder = pool.begin().await.expect("public Run lock witness");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("holder backend");
    let _: Uuid = sqlx::query_scalar("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(fixture.platform.tenant.as_uuid())
        .bind(fixture.run)
        .fetch_one(&mut *holder)
        .await
        .expect("hold the genuine public Run");
    let down_pool = (*pool).clone();
    let down = tokio::spawn(async move { MIGRATOR.undo(&down_pool, BEFORE_RUN_DATA_LIMIT).await });
    crate::support::eventually("down waits on the declared first Run lock", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_locks waiting \
             JOIN pg_stat_activity activity ON activity.pid=waiting.pid \
             WHERE waiting.locktype='relation' AND waiting.relation='runs'::regclass \
               AND waiting.mode='AccessExclusiveLock' AND NOT waiting.granted \
               AND activity.datname=current_database() \
               AND $1=ANY(pg_blocking_pids(waiting.pid)))",
        )
        .bind(holder_pid)
        .fetch_one(pool)
        .await
        .expect("actual Run lock wait")
        .then_some(())
    })
    .await;
    holder.rollback().await.expect("release public Run witness");
    tokio::time::timeout(std::time::Duration::from_secs(5), down)
        .await
        .expect("downgrade clears the witnessed lock wait")
        .expect("downgrade task")
        .expect("no retained new reason permits schema downgrade");
    assert_eq!(migration_version(pool).await, BEFORE_RUN_DATA_LIMIT);
    assert!(!constraint_definition(pool).await.contains("RUN_DATA_LIMIT"));
    assert_eq!(
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT (SELECT count(*) FROM runs WHERE tenant_id=$1 AND id=$2), \
                    (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2)",
        )
        .bind(fixture.platform.tenant.as_uuid())
        .bind(fixture.run)
        .fetch_one(pool)
        .await
        .expect("ordinary public history survives down"),
        before
    );

    migrate(pool)
        .await
        .expect("reapply run-data-limit vocabulary");
    assert_eq!(migration_version(pool).await, head_version());
    let definition = constraint_definition(pool).await;
    assert!(definition.contains("RUN_DATA_LIMIT"));
    assert!(definition.contains("IS TRUE"));
}

#[tokio::test]
async fn retained_static_run_data_limit_scope_refuses_downgrade() {
    let fixture = fixture().await;
    let candidate = Projection::drain(
        "DRAINING",
        "RUN_DATA_LIMIT",
        None,
        Some("RUN_DATA_LIMIT"),
        1,
    );
    sqlx::query(
        "UPDATE iteration_scopes SET lifecycle=$4,drain_kind=$5,drain_iteration=$6, \
         drain_failure_code=$7,drain_sequence=$8,drain_outbox_id=$9, \
         updated_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
    )
    .bind(fixture.platform.tenant.as_uuid())
    .bind(fixture.run)
    .bind(fixture.scope)
    .bind(candidate.lifecycle)
    .bind(candidate.drain_kind)
    .bind(candidate.drain_iteration)
    .bind(candidate.drain_failure_code)
    .bind(candidate.drain_sequence)
    .bind(candidate.drain_outbox_id)
    .execute(&fixture.platform.api.db.pool)
    .await
    .expect("explicit static RUN_DATA_LIMIT scope fixture");

    retained_reason(
        MIGRATOR
            .undo(&fixture.platform.api.db.pool, BEFORE_RUN_DATA_LIMIT)
            .await
            .expect_err("retained scope reason refuses down"),
    );
    assert_eq!(
        migration_version(&fixture.platform.api.db.pool).await,
        RUN_DATA_LIMIT_VERSION
    );
    let retained: (String, Option<String>) = sqlx::query_as(
        "SELECT lifecycle,drain_kind FROM iteration_scopes \
         WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
    )
    .bind(fixture.platform.tenant.as_uuid())
    .bind(fixture.run)
    .bind(fixture.scope)
    .fetch_one(&fixture.platform.api.db.pool)
    .await
    .expect("retained static scope survives refusal");
    assert_eq!(retained, ("DRAINING".into(), Some("RUN_DATA_LIMIT".into())));
}

#[tokio::test]
async fn retained_static_run_data_limit_journal_refuses_downgrade() {
    let fixture = fixture().await;
    let tenant = fixture.platform.tenant.as_uuid();
    let mut transaction = fixture
        .platform
        .api
        .db
        .pool
        .begin()
        .await
        .expect("static journal fixture");
    let sequence: i64 = sqlx::query_scalar(
        "SELECT run_version + 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(fixture.run)
    .fetch_one(&mut *transaction)
    .await
    .expect("lock public Run before static journal append");
    sqlx::query(
        "INSERT INTO run_events(tenant_id,run_id,seq,event_type,node,detail,actor,occurred_at) \
         VALUES($1,$2,$3,'collection.drain_started','items', \
         jsonb_build_object('reason','run_data_limit'),'schema-constraint-fixture', \
         clock_timestamp())",
    )
    .bind(tenant)
    .bind(fixture.run)
    .bind(sequence)
    .execute(&mut *transaction)
    .await
    .expect("explicit static RUN_DATA_LIMIT journal fixture");
    sqlx::query(
        "UPDATE runs SET run_version=$3,updated_at=clock_timestamp() \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(fixture.run)
    .bind(sequence)
    .execute(&mut *transaction)
    .await
    .expect("static journal version projection");
    transaction
        .commit()
        .await
        .expect("commit retained static journal");

    retained_reason(
        MIGRATOR
            .undo(&fixture.platform.api.db.pool, BEFORE_RUN_DATA_LIMIT)
            .await
            .expect_err("retained journal reason refuses down"),
    );
    assert_eq!(
        migration_version(&fixture.platform.api.db.pool).await,
        RUN_DATA_LIMIT_VERSION
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
         AND event_type='collection.drain_started' \
         AND detail->>'reason'='run_data_limit'",
    )
    .bind(tenant)
    .bind(fixture.run)
    .fetch_one(&fixture.platform.api.db.pool)
    .await
    .expect("retained static event survives refusal");
    assert_eq!(retained, 1);
}

#[tokio::test]
async fn downgrade_owns_run_before_a_new_reason_writer_and_the_writer_rolls_back() {
    let fixture = fixture().await;
    let pool = &fixture.platform.api.db.pool;
    let tenant = fixture.platform.tenant.as_uuid();
    let before: (i64, i64) = sqlx::query_as(
        r#"SELECT run_version,
           (SELECT count(*) FROM run_events event
             WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id
               AND event.event_type='collection.drain_started'
               AND event.detail->>'reason'='run_data_limit')
           FROM runs run WHERE run.tenant_id=$1 AND run.id=$2"#,
    )
    .bind(tenant)
    .bind(fixture.run)
    .fetch_one(pool)
    .await
    .expect("pre-queue Run and journal projection");
    assert_eq!(before.1, 0);
    assert_eq!(migration_version(pool).await, head_version());

    let mut gate = pool.begin().await.expect("scope table gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("capture gate PID before its held lock query");
    sqlx::query("LOCK TABLE iteration_scopes IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .expect("G holds only the scope table gate");

    let mut down_connection = pool.acquire().await.expect("dedicated downgrade session");
    let down_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *down_connection)
        .await
        .expect("capture downgrade PID before migration locks");
    // SQLx's connection-borrowing migration future is not Send. Drive it on this runtime from
    // a blocking helper while keeping the captured, dedicated backend identity unchanged.
    let runtime = tokio::runtime::Handle::current();
    let down = tokio::task::spawn_blocking(move || {
        runtime.block_on(async move {
            MIGRATOR
                .undo(&mut *down_connection, BEFORE_RUN_DATA_LIMIT)
                .await
        })
    });
    crate::support::eventually("downgrade owns Run before waiting for scope", || async {
        sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(
                 SELECT 1 FROM pg_stat_activity down
                 WHERE down.pid=$2 AND down.datname=current_database()
                   AND down.wait_event_type='Lock'
                   AND $1=ANY(pg_blocking_pids(down.pid)))
               AND EXISTS(
                 SELECT 1 FROM pg_locks
                 WHERE pid=$2 AND relation='runs'::regclass
                   AND mode='AccessExclusiveLock' AND granted)
               AND EXISTS(
                 SELECT 1 FROM pg_locks
                 WHERE pid=$2 AND relation='iteration_scopes'::regclass
                   AND mode='AccessExclusiveLock' AND NOT granted)"#,
        )
        .bind(gate_pid)
        .bind(down_pid)
        .fetch_one(pool)
        .await
        .expect("downgrade Run/scope lock witness")
        .then_some(())
    })
    .await;

    let mut writer = pool.begin().await.expect("new-reason writer transaction");
    let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .expect("capture writer PID before its Run lock query");
    let run = fixture.run;
    let scope = fixture.scope;
    let drain_outbox = Uuid::new_v4();
    let writer = tokio::spawn(async move {
        let sequence: i64 = sqlx::query_scalar(
            r#"SELECT run_version + 1 FROM runs
               WHERE tenant_id=$1 AND id=$2 FOR UPDATE"#,
        )
        .bind(tenant)
        .bind(run)
        .fetch_one(&mut *writer)
        .await?;
        let updated = sqlx::query(
            r#"UPDATE iteration_scopes SET lifecycle='DRAINING',
               drain_kind='RUN_DATA_LIMIT',drain_iteration=NULL,
               drain_failure_code='RUN_DATA_LIMIT',drain_sequence=$4,
               drain_outbox_id=$5,updated_at=clock_timestamp()
               WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3"#,
        )
        .bind(tenant)
        .bind(run)
        .bind(scope)
        .bind(sequence)
        .bind(drain_outbox)
        .execute(&mut *writer)
        .await;
        if let Err(error) = updated {
            writer
                .rollback()
                .await
                .expect("rollback rejected new-reason writer");
            return Err(error);
        }
        sqlx::query(
            r#"INSERT INTO run_events
               (tenant_id,run_id,seq,event_type,node,detail,actor,occurred_at)
               VALUES($1,$2,$3,'collection.drain_started','items',
                 jsonb_build_object('reason','run_data_limit'),'schema-queue-fixture',
                 clock_timestamp())"#,
        )
        .bind(tenant)
        .bind(run)
        .bind(sequence)
        .execute(&mut *writer)
        .await?;
        sqlx::query(
            r#"UPDATE runs SET run_version=$3,updated_at=clock_timestamp()
               WHERE tenant_id=$1 AND id=$2"#,
        )
        .bind(tenant)
        .bind(run)
        .bind(sequence)
        .execute(&mut *writer)
        .await?;
        writer.commit().await?;
        Ok::<(), sqlx::Error>(())
    });

    observe_down_before_writer_queue(pool, [gate_pid, down_pid, writer_pid]).await;
    gate.rollback().await.expect("release scope table gate");
    tokio::time::timeout(std::time::Duration::from_secs(5), down)
        .await
        .expect("downgrade clears the scope gate")
        .expect("downgrade task")
        .expect("down wins before the queued new-reason writer");
    let writer_error = tokio::time::timeout(std::time::Duration::from_secs(5), writer)
        .await
        .expect("writer clears the downgrade queue")
        .expect("writer task")
        .expect_err("the restored old constraint rejects the new reason");
    let (code, constraint) = crate::support::db_error(writer_error);
    assert_eq!(code, "23514");
    assert_eq!(constraint.as_deref(), Some("iteration_scopes_drain_reason"));
    assert_eq!(migration_version(pool).await, BEFORE_RUN_DATA_LIMIT);
    assert!(!constraint_definition(pool).await.contains("RUN_DATA_LIMIT"));

    let after: (i64, i64) = sqlx::query_as(
        r#"SELECT run_version,
           (SELECT count(*) FROM run_events event
             WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id
               AND event.event_type='collection.drain_started'
               AND event.detail->>'reason'='run_data_limit')
           FROM runs run WHERE run.tenant_id=$1 AND run.id=$2"#,
    )
    .bind(tenant)
    .bind(fixture.run)
    .fetch_one(pool)
    .await
    .expect("post-queue Run and journal projection");
    assert_eq!(
        after, before,
        "rejected writer retains no journal or revision"
    );
    let unchanged_scope: bool = sqlx::query_scalar(
        r#"SELECT lifecycle='OPEN'
           AND drain_kind IS NULL AND drain_iteration IS NULL
           AND drain_failure_code IS NULL AND drain_sequence IS NULL
           AND drain_outbox_id IS NULL
           FROM iteration_scopes
           WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3"#,
    )
    .bind(tenant)
    .bind(fixture.run)
    .bind(fixture.scope)
    .fetch_one(pool)
    .await
    .expect("rejected writer retains no scope projection");
    assert!(unchanged_scope);
}

//! The migration of the resets that wait for their source (`T018-057`, `20270108000000_pending_resets`) against a real
//! PostgreSQL: its `down` refuses while anything it added is in use, and on a clean schema it restores the CHECKs and the
//! trigger it replaced, after which the `up` applies again. The style of the rollback tests of `migration.rs`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_infra_postgres::migrate;
use serde_json::json;
use uuid::Uuid;

use crate::firing::{Fixture, fixture};
use crate::migration::undo_to;

const THIS: i64 = 20_270_108_000_000;
/// The migration before it: where a rollback of this one alone stops.
const PREVIOUS: i64 = 20_270_107_000_000;

const CHECK_VIOLATION: &str = "23514";

fn code<T>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(_) => panic!("the statement was expected to fail"),
        Err(sqlx::Error::Database(e)) => e.code().map(|c| c.to_string()).unwrap_or_default(),
        Err(other) => panic!("not a database error: {other}"),
    }
}

/// An entry of the register as the schema takes it, with the columns a case is about.
async fn entry(
    f: &Fixture,
    source: Uuid,
    policy: &str,
    state: &str,
    failure_reason: Option<&str>,
    acknowledged: bool,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO run_resets \
         (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, reapply, business_id_policy, \
          source_policy, state, failure_reason, actor, reason, created_at, completed_at, reapply_report, \
          acknowledged_reexecution) \
         VALUES ($1, $2, $3, $4, 1, $5, 'NONE', 'KEEP', $6, $7, $8, 'alice', 'a reason of the requester', now(), \
                 CASE WHEN $7 = 'PENDING_SOURCE' THEN NULL ELSE now() END, '[]', $9)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(id)
    .bind(source)
    .bind(vec![0u8; 32])
    .bind(policy)
    .bind(state)
    .bind(failure_reason)
    .bind(acknowledged)
    .execute(&f.w.db.pool)
    .await
    .map(|_| id)
}

/// Removes an entry the way an operator would after reading it: a waiting one is ended first (the schema never purges one
/// that waits), then the row goes.
async fn remove(f: &Fixture, id: Uuid) {
    sqlx::query(
        "UPDATE run_resets SET state = 'FAILED', failure_reason = 'ADMISSION_REFUSED', completed_at = now() \
         WHERE id = $1 AND state = 'PENDING_SOURCE'",
    )
    .bind(id)
    .execute(&f.w.db.pool)
    .await
    .expect("end a waiting entry");
    sqlx::query("DELETE FROM run_resets WHERE id = $1")
        .bind(id)
        .execute(&f.w.db.pool)
        .await
        .expect("remove the entry");
}

#[tokio::test]
async fn the_rollback_is_refused_for_each_thing_the_migration_added_while_it_is_in_use() {
    let f = fixture().await;
    let source = f.w.start(f.workflow, &json!({})).await.as_uuid();

    // each case isolates one clause of the guard: the others are at their old values
    let entries: [(&str, &str, &str, Option<&str>, bool); 4] = [
        (
            "a reset that waits",
            "REQUIRE_TERMINAL",
            "PENDING_SOURCE",
            None,
            false,
        ),
        (
            "a request that asked to cancel",
            "CANCEL_SOURCE",
            "FAILED",
            Some("ADMISSION_REFUSED"),
            false,
        ),
        (
            "a failure the migration named",
            "REQUIRE_TERMINAL",
            "FAILED",
            Some("AUTHORITY_REVOKED"),
            false,
        ),
        (
            "an acknowledgement",
            "REQUIRE_TERMINAL",
            "FAILED",
            Some("ADMISSION_REFUSED"),
            true,
        ),
    ];
    for (what, policy, state, reason, acknowledged) in entries {
        let id = entry(&f, source, policy, state, reason, acknowledged)
            .await
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        let refused = undo_to(&f.w.db, PREVIOUS)
            .await
            .expect_err(&format!("{what} is evidence"));
        assert!(
            format!("{refused:?}").contains("retained"),
            "{what}: {refused:?}"
        );
        // nothing was lost: the column and the row are there
        let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE id = $1")
            .bind(id)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the entry");
        assert_eq!(kept, 1, "{what}");
        remove(&f, id).await;
    }

    // a limit and a ceiling of the new resource
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'pending_resets', 3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit");
    let refused = undo_to(&f.w.db, PREVIOUS)
        .await
        .expect_err("a limit is an operator's");
    assert!(format!("{refused:?}").contains("retained"), "{refused:?}");
    sqlx::query("DELETE FROM quota_limits WHERE resource = 'pending_resets'")
        .execute(&f.w.db.pool)
        .await
        .expect("the operator removes the limit");

    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'pending_resets', 32, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a ceiling");
    let refused = undo_to(&f.w.db, PREVIOUS)
        .await
        .expect_err("a ceiling is an operator's");
    assert!(format!("{refused:?}").contains("retained"), "{refused:?}");
    sqlx::query("DELETE FROM quota_ceilings WHERE resource = 'pending_resets'")
        .execute(&f.w.db.pool)
        .await
        .expect("the operator removes the ceiling");

    // and with all of it gone it goes
    undo_to(&f.w.db, PREVIOUS)
        .await
        .expect("the rollback of a schema that uses none of it");
    migrate(&f.w.db.pool).await.expect("forward again");
}

/// The definitions of the three lists of resources, as PostgreSQL stores them.
async fn quota_checks(f: &Fixture) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conname IN ('quota_ceilings_resource_valid', 'quota_limits_resource_valid', \
                           'quota_limits_tenant_scoped') ORDER BY conname",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the checks")
}

/// The quoted names of a definition: the members of the list a CHECK holds.
fn names(definition: &str) -> std::collections::BTreeSet<String> {
    definition
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

async fn definition(f: &Fixture, query: &'static str) -> String {
    sqlx::query_scalar(query)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("a definition")
}

const FAILURE_CHECK: &str = "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'run_resets_failure_reason_valid'";
const GUARD: &str = "SELECT pg_get_functiondef('run_resets_guard'::regproc)";
const HAS_COLUMN: &str = "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
     WHERE table_name = 'run_resets' AND column_name = 'acknowledged_reexecution')";
const HAS_INDEX: &str = "SELECT to_regclass('public.run_resets_pending_namespace_idx') IS NOT NULL";

#[tokio::test]
async fn on_a_clean_schema_the_rollback_restores_the_checks_and_the_trigger_and_the_up_applies_again()
 {
    let f = fixture().await;
    let source = f.w.start(f.workflow, &json!({})).await.as_uuid();
    // an entry as the migration before it allowed: it is not in the way of the rollback
    entry(
        &f,
        source,
        "REQUIRE_TERMINAL",
        "FAILED",
        Some("RUN_PURGED"),
        false,
    )
    .await
    .expect("an old-style entry");

    let (checks, failure, guard) = (
        quota_checks(&f).await,
        definition(&f, FAILURE_CHECK).await,
        definition(&f, GUARD).await,
    );
    assert!(
        checks
            .iter()
            .all(|(_, def)| def.contains("'pending_resets'"))
    );
    assert!(failure.contains("AUTHORITY_REVOKED"));
    assert!(guard.contains("acknowledged_reexecution"));
    let column: bool = sqlx::query_scalar(HAS_COLUMN)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("column");
    assert!(column, "the column is there before the rollback");

    undo_to(&f.w.db, PREVIOUS).await.expect("the rollback");

    // the column and the index are gone
    let column: bool = sqlx::query_scalar(HAS_COLUMN)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("column");
    let index: bool = sqlx::query_scalar(HAS_INDEX)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("index");
    assert!(
        (column, index) == (false, false),
        "column {column}, index {index}"
    );
    // the three lists are what they were: the same without the new name
    let undone = quota_checks(&f).await;
    assert_eq!(undone.len(), 3);
    for ((name, was), (undone_name, now)) in checks.iter().zip(&undone) {
        assert_eq!(name, undone_name);
        assert!(!now.contains("pending_resets"), "{name}: {now}");
        let mut expected = names(was);
        assert!(expected.remove("pending_resets"), "{name}: {was}");
        assert_eq!(
            names(now),
            expected,
            "{name}: the list of the migration before, no more and no less"
        );
    }
    // the reasons of the migration before, and the trigger without the acknowledgement
    let failure_now = definition(&f, FAILURE_CHECK).await;
    for reason in [
        "AUTHORITY_REVOKED",
        "REEXECUTION_NOT_ACKNOWLEDGED",
        "REAPPLY_TOO_MANY",
    ] {
        assert!(!failure_now.contains(reason), "{reason}: {failure_now}");
    }
    assert!(failure_now.contains("RUN_PURGED"));
    let guard_now = definition(&f, GUARD).await;
    assert!(!guard_now.contains("acknowledged_reexecution"));
    assert!(guard_now.contains("what a reset is never changes"));
    // what it refuses again: a name it does not know, in the lists and in the register
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
                 VALUES ($1, $2, 'pending_resets', 3, 'admin')",
            )
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(f.w.namespace.id().as_uuid())
            .execute(&f.w.db.pool)
            .await
        ),
        CHECK_VIOLATION,
        "the previous list is back"
    );
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO run_resets \
                 (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, reapply, \
                  business_id_policy, source_policy, state, failure_reason, actor, reason, created_at, completed_at) \
                 VALUES ($1, $2, $3, $4, 1, $5, 'NONE', 'KEEP', 'REQUIRE_TERMINAL', 'FAILED', 'AUTHORITY_REVOKED', \
                         'alice', 'a reason of the requester', now(), now())",
            )
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(f.w.namespace.id().as_uuid())
            .bind(Uuid::now_v7())
            .bind(source)
            .bind(vec![0u8; 32])
            .execute(&f.w.db.pool)
            .await
        ),
        CHECK_VIOLATION,
        "the previous reasons are back"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(THIS)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recorded");
    assert_eq!(recorded, 0, "the rollback is recorded");

    // the up applies again, to a register that has entries, and gives back what the first time gave
    migrate(&f.w.db.pool).await.expect("forward again");
    assert_eq!(quota_checks(&f).await, checks);
    assert_eq!(definition(&f, FAILURE_CHECK).await, failure);
    assert_eq!(definition(&f, GUARD).await, guard);
    let column: bool = sqlx::query_scalar(HAS_COLUMN)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("column");
    let index: bool = sqlx::query_scalar(HAS_INDEX)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("index");
    assert!(column && index);
    // the old entry kept its values, and the new column says it was not acknowledged
    let kept: (String, bool) = sqlx::query_as(
        "SELECT failure_reason, acknowledged_reexecution FROM run_resets WHERE source_run_id = $1",
    )
    .bind(source)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the old entry");
    assert_eq!(kept, ("RUN_PURGED".to_owned(), false));
}

//! The migration of the expiry of a reset that waits (`T018-057`, `20270113000000_reset_expiry`, D2059) against a real
//! PostgreSQL: the list of the reasons a reset can fail with admits `SOURCE_DID_NOT_END` and stays closed; its `down`
//! refuses while an entry that expired exists (it is evidence of a request that cancelled a run and made no replacement) and,
//! on a schema without one, restores the list of the migration before, after which the `up` applies again. The style of
//! `pending_resets_migration.rs`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::json;

use crate::firing::{Fixture, fixture};
use crate::migration::undo_to;
use crate::pending_resets_migration::{CHECK_VIOLATION, code, entry};

/// This migration: the one version a renumbering at merge changes in this file.
const THIS: i64 = 20_270_113_000_000;

/// The migration before it, read from the list the migrator embeds: where a rollback of this one alone stops.
fn previous() -> i64 {
    MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version < THIS)
        .map(|m| m.version)
        .max()
        .expect("a migration before this one")
}

const FAILURE_CHECK: &str = "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'run_resets_failure_reason_valid'";

async fn definition(f: &Fixture) -> String {
    sqlx::query_scalar(FAILURE_CHECK)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the check")
}

#[tokio::test]
async fn the_list_of_reasons_admits_the_expiry_and_stays_closed() {
    let f = fixture().await;
    let source = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let failed = |reason: &'static str| {
        let f = &f;
        async move { entry(f, source, "CANCEL_SOURCE", "FAILED", Some(reason), false).await }
    };
    // the new reason, and every reason the migrations before it admit
    for reason in [
        "SOURCE_DID_NOT_END",
        "BOUNDARY_NO_LONGER_ELIGIBLE",
        "ADMISSION_REFUSED",
        "BUSINESS_ID_IN_USE",
        "CONCURRENCY_PERMIT_UNAVAILABLE",
        "RUN_PURGED",
        "AUTHORITY_REVOKED",
        "REEXECUTION_NOT_ACKNOWLEDGED",
        "REAPPLY_TOO_MANY",
    ] {
        failed(reason)
            .await
            .unwrap_or_else(|error| panic!("{reason} is a reason: {error}"));
    }
    // and nothing else: not a name that is like it, not one in another case, not an empty one
    for reason in [
        "SOURCE_DID_NOT_END ",
        "source_did_not_end",
        "SOURCE_NEVER_ENDED",
        "EXPIRED",
        "",
    ] {
        assert_eq!(
            code(failed(reason).await),
            CHECK_VIOLATION,
            "{reason:?} is not a reason"
        );
    }
    // (That a reason belongs only to an entry that failed, and a run only to one that was created, is decided by
    // `run_resets_failure_only_when_failed` and `run_resets_run_only_when_created` of `20270107000000_run_resets`, not by
    // this migration: they are not asserted here.)
}

#[test]
fn this_migration_is_the_one_the_rollbacks_of_this_file_start_from() {
    // a renumbering at merge moves THIS and nothing else of this file: the migration before it is read from the list
    let this: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version == THIS)
        .collect();
    assert_eq!(this.len(), 1, "THIS names one forward migration");
    assert!(this[0].description.contains("reset expiry"));
    assert!(previous() < THIS);
}

#[tokio::test]
async fn the_rollback_is_refused_while_an_entry_that_expired_exists_and_restores_the_list_when_none_does()
 {
    let f = fixture().await;
    let source = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let before = definition(&f).await;
    assert!(before.contains("SOURCE_DID_NOT_END"));

    // an entry that expired is evidence: the rollback does not destroy it
    let expired = entry(
        &f,
        source,
        "CANCEL_SOURCE",
        "FAILED",
        Some("SOURCE_DID_NOT_END"),
        true,
    )
    .await
    .expect("an entry that expired");
    let refused = undo_to(&f.w.db, previous())
        .await
        .expect_err("an entry that expired is evidence");
    assert!(
        format!("{refused:?}").contains("expired are retained"),
        "{refused:?}"
    );
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE id = $1")
        .bind(expired)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry");
    assert_eq!(kept, 1, "nothing was lost");
    assert_eq!(definition(&f).await, before, "and the list is as it was");

    // an entry that failed for another reason is not in the way, and a rollback after the operator removed the one that
    // expired restores the list of the migration before
    let other = entry(
        &f,
        source,
        "CANCEL_SOURCE",
        "FAILED",
        Some("AUTHORITY_REVOKED"),
        true,
    )
    .await
    .expect("an entry that failed for another reason");
    sqlx::query("DELETE FROM run_resets WHERE id = $1")
        .bind(expired)
        .execute(&f.w.db.pool)
        .await
        .expect("the operator removes the entry that expired");
    undo_to(&f.w.db, previous())
        .await
        .expect("the rollback of a schema that holds none");
    let after = definition(&f).await;
    assert!(!after.contains("SOURCE_DID_NOT_END"), "{after}");
    for reason in [
        "BOUNDARY_NO_LONGER_ELIGIBLE",
        "ADMISSION_REFUSED",
        "BUSINESS_ID_IN_USE",
        "CONCURRENCY_PERMIT_UNAVAILABLE",
        "RUN_PURGED",
        "AUTHORITY_REVOKED",
        "REEXECUTION_NOT_ACKNOWLEDGED",
        "REAPPLY_TOO_MANY",
    ] {
        assert!(after.contains(reason), "{reason}: {after}");
    }
    assert_eq!(
        code(
            entry(
                &f,
                source,
                "CANCEL_SOURCE",
                "FAILED",
                Some("SOURCE_DID_NOT_END"),
                false
            )
            .await
        ),
        CHECK_VIOLATION,
        "the previous list is back"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(THIS)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recorded");
    assert_eq!(recorded, 0, "the rollback is recorded");
    // the entry that failed for another reason is untouched
    let reason: String = sqlx::query_scalar("SELECT failure_reason FROM run_resets WHERE id = $1")
        .bind(other)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the other entry");
    assert_eq!(reason, "AUTHORITY_REVOKED");

    // the up applies again and gives back what the first time gave
    migrate(&f.w.db.pool).await.expect("forward again");
    assert_eq!(definition(&f).await, before);
    entry(
        &f,
        source,
        "CANCEL_SOURCE",
        "FAILED",
        Some("SOURCE_DID_NOT_END"),
        false,
    )
    .await
    .expect("the reason is admitted again");
}

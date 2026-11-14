//! The migration of the count of failed selection rounds (`T018-046`, `20270116000000_batch_selection_failures`, G2093, D2197)
//! against a real PostgreSQL: the column is a count that defaults to zero for a job an older engine wrote, it holds a count and
//! nothing else, and no rule of the table stops an older engine that does not know it; the runtime role may write it; the `down` refuses while a
//! job has failed rounds on record (they are the evidence of what the platform did) and, on a schema without any, takes the column
//! away and leaves every job, after which the `up` applies again. The style of `reset_expiry_migration.rs`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_infra_postgres::{MIGRATOR, migrate};
use uuid::Uuid;

use crate::batch_executor::{create, everything, runs_started, write};
use crate::firing::{Fixture, fixture};
use crate::migration::undo_to;
use crate::pending_resets_migration::{CHECK_VIOLATION, code};
use sagat_domain::batch::Action;

/// This migration: the one version a renumbering at merge changes in this file.
const THIS: i64 = 20_270_116_000_000;

/// The migration before it, read from the list the migrator embeds: where a rollback of this one alone stops.
fn previous() -> i64 {
    MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version < THIS)
        .map(|m| m.version)
        .max()
        .expect("a migration before this one")
}

#[test]
fn this_migration_is_the_one_the_rollbacks_of_this_file_start_from() {
    // a renumbering at merge moves THIS and nothing else of this file: the migration before it is read from the list
    let this: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version == THIS)
        .collect();
    assert_eq!(this.len(), 1, "THIS names one forward migration");
    assert!(this[0].description.contains("batch selection failures"));
    assert!(previous() < THIS);
}

/// `(data type, nullable, default)` of the column, from the catalog; none when there is no such column.
async fn column(f: &Fixture) -> Option<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT data_type, is_nullable, column_default FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'batch_jobs' AND column_name = 'selection_failures'",
    )
    .fetch_optional(&f.w.db.pool)
    .await
    .expect("the catalog")
}

async fn make_a_job(f: &Fixture, key: &str) -> Uuid {
    runs_started(f, 1).await;
    create(f, write(Action::Cancel, everything()), key).await.id
}

/// Catches: a column that is nullable or has no default (a job an older engine wrote would read as nothing), a count below
/// zero, a rule that refuses the write of an older engine that does not know the column, and a privilege the runtime role
/// lacks on the new column.
#[tokio::test]
async fn the_column_is_a_count_that_starts_at_zero_and_does_not_stop_an_older_engine() {
    let f = fixture().await;
    assert_eq!(
        column(&f).await,
        Some(("smallint".to_owned(), "NO".to_owned(), Some("0".to_owned()))),
        "a count that every job has, zero for a job written without it"
    );
    let job = make_a_job(&f, "selection-migration-column").await;
    let count = |f: &Fixture| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i16>("SELECT selection_failures FROM batch_jobs WHERE id = $1")
                .bind(job)
                .fetch_one(&pool)
                .await
                .expect("the count")
        }
    };
    assert_eq!(count(&f).await, 0, "a job made by this build has none");

    // a count and nothing below zero
    assert_eq!(
        code(
            sqlx::query("UPDATE batch_jobs SET selection_failures = -1 WHERE id = $1")
                .bind(job)
                .execute(&f.w.db.pool)
                .await
        ),
        CHECK_VIOLATION
    );
    // the guard of the table lets the count move (it is progress) and still refuses a change of what the job is
    sqlx::query("UPDATE batch_jobs SET selection_failures = 3 WHERE id = $1")
        .bind(job)
        .execute(&f.w.db.pool)
        .await
        .expect("a job that is selecting may have failed rounds");
    assert_eq!(count(&f).await, 3);
    let guard =
        sqlx::query("UPDATE batch_jobs SET reason = 'a reason that was never given' WHERE id = $1")
            .bind(job)
            .execute(&f.w.db.pool)
            .await;
    assert!(
        guard.is_err(),
        "what a batch job is still never changes: {guard:?}"
    );
    // No rule of the table ties the count to the state of the job: an older engine that takes the shard over picks its page without
    // knowing the column, and its write is not refused (that a pick of this build resets the count is held by the tests of the
    // executor, `batch_selection_failures::`)
    sqlx::query("UPDATE batch_jobs SET state = 'RUNNING' WHERE id = $1")
        .bind(job)
        .execute(&f.w.db.pool)
        .await
        .expect("the pick of an engine that does not know the column is not refused");
    assert_eq!(count(&f).await, 3, "and leaves the count where it was");

    // the runtime role, which is what the executor connects as, writes it (no grant moved: the table-wide one covers it)
    let other = make_a_job(&f, "selection-migration-role").await;
    let runtime = f.w.db.runtime_pool().await;
    let moved = sqlx::query("UPDATE batch_jobs SET selection_failures = 2 WHERE id = $1")
        .bind(other)
        .execute(&runtime)
        .await
        .expect("the runtime role writes the count")
        .rows_affected();
    assert_eq!(moved, 1);
    let read: i16 = sqlx::query_scalar("SELECT selection_failures FROM batch_jobs WHERE id = $1")
        .bind(other)
        .fetch_one(&runtime)
        .await
        .expect("and reads it");
    assert_eq!(read, 2);
}

/// The `down` refuses while a job that is still selecting holds failed rounds (it would lose its progress to the bound: an older
/// engine would take it up as if nothing had failed), and only then. A job that ended `SELECTION_FAILED` keeps its count as
/// evidence and its reason says the rest, so it does not stand in the way of a rollback for the whole retention of the job; a job
/// that is cancelling or cancelled does not either. Catches: a guard that refuses for a job that is over (the rollback would be
/// refused for the retention of every job that ever failed), one that does not refuse for a job that is selecting, one that loses
/// a job, the reason of a job that failed, and an `up` that does not apply again.
#[tokio::test]
async fn the_rollback_is_refused_while_a_job_that_is_selecting_has_failed_rounds_and_takes_only_the_column_when_none_does()
 {
    let f = fixture().await;
    let failed = make_a_job(&f, "selection-migration-failed").await;
    let waiting = make_a_job(&f, "selection-migration-waiting").await;
    // a job that ended `SELECTION_FAILED` keeps the count of the rounds that failed
    sqlx::query(
        "UPDATE batch_jobs SET state = 'FAILED', failure_reason = 'SELECTION_FAILED', finished_at = clock_timestamp(), \
                selection_failures = 11 WHERE id = $1",
    )
    .bind(failed)
    .execute(&f.w.db.pool)
    .await
    .expect("a job that failed its selection");
    // and one that is selecting has four on record
    sqlx::query("UPDATE batch_jobs SET selection_failures = 4 WHERE id = $1")
        .bind(waiting)
        .execute(&f.w.db.pool)
        .await
        .expect("a job that waits for its next round");

    let refused = undo_to(&f.w.db, previous())
        .await
        .expect_err("a job that is selecting would lose the progress of its bound");
    assert!(
        format!("{refused:?}").contains("still selecting"),
        "{refused:?}"
    );
    assert!(column(&f).await.is_some(), "the column is where it was");
    let count: i16 = sqlx::query_scalar("SELECT selection_failures FROM batch_jobs WHERE id = $1")
        .bind(waiting)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the count");
    assert_eq!(count, 4, "nothing was lost");

    // the job that waited has ended (a cancel keeps its count too: it is not selecting any more), and the rollback takes the column
    sqlx::query("UPDATE batch_jobs SET state = 'CANCELLING' WHERE id = $1")
        .bind(waiting)
        .execute(&f.w.db.pool)
        .await
        .expect("the job is cancelled while it waits");
    undo_to(&f.w.db, previous())
        .await
        .expect("the rollback of a schema whose only counts are on jobs that are not selecting");
    assert_eq!(column(&f).await, None, "the column is gone");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM batch_jobs WHERE id = ANY($1)")
        .bind(vec![failed, waiting])
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the jobs");
    assert_eq!(
        jobs, 2,
        "every job is where it was: an older engine reads them"
    );
    let reason: Option<String> =
        sqlx::query_scalar("SELECT failure_reason FROM batch_jobs WHERE id = $1")
            .bind(failed)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the reason");
    assert_eq!(
        reason.as_deref(),
        Some("SELECTION_FAILED"),
        "the job that failed still says why: the count was evidence of how many rounds, the reason is the record"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(THIS)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recorded");
    assert_eq!(recorded, 0, "the rollback is recorded");

    // the up applies again and gives every job the count an older engine never wrote
    migrate(&f.w.db.pool).await.expect("forward again");
    assert_eq!(
        column(&f).await,
        Some(("smallint".to_owned(), "NO".to_owned(), Some("0".to_owned())))
    );
    let counts: Vec<i16> =
        sqlx::query_scalar("SELECT selection_failures FROM batch_jobs WHERE id = ANY($1)")
            .bind(vec![failed, waiting])
            .fetch_all(&f.w.db.pool)
            .await
            .expect("the counts");
    assert_eq!(
        counts,
        vec![0, 0],
        "a job written before the column has none"
    );
}
/// `next_slot_at` means two things since this migration, and the catalog says both (D2198): the rate slot of a running job and the
/// end of the wait after a failed selection round. The `down` takes the comment away with the column. Catches: a column that keeps
/// its old meaning in the catalog, and a `down` that leaves the comment of the migration it undoes.
#[tokio::test]
async fn the_catalog_says_what_the_slot_of_a_job_means_and_the_rollback_takes_the_comment_away() {
    let f = fixture().await;
    let comment = |f: &Fixture| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT col_description('batch_jobs'::regclass, a.attnum) FROM pg_attribute a \
                 WHERE a.attrelid = 'batch_jobs'::regclass AND a.attname = 'next_slot_at'",
            )
            .fetch_one(&pool)
            .await
            .expect("the comment")
        }
    };
    let said = comment(&f).await.expect("a comment on the slot");
    assert!(
        said.contains("rate slot") && said.contains("failed selection round"),
        "both meanings: {said}"
    );
    undo_to(&f.w.db, previous())
        .await
        .expect("the rollback of a schema with no count on record");
    assert_eq!(
        comment(&f).await,
        None,
        "the comment goes with the migration"
    );
    migrate(&f.w.db.pool).await.expect("forward again");
    assert!(comment(&f).await.is_some(), "and comes back with it");
}

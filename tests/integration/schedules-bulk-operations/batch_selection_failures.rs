//! A selection round that always fails is bounded (`G2093`, `D2197`, `FR-018-029`): the failure of a round is counted in the
//! database, the job waits a backoff between two rounds, and the eleventh failure in a row ends the job `FAILED` with
//! `SELECTION_FAILED` by the writes `AUTHORITY_REVOKED` uses. A round that picks starts the count again, the count and the
//! wait survive a restart and a move of the shard lease, and a job that waits neither starves the jobs behind it in the scan nor
//! is lost by it. Real PostgreSQL, the real executor, the real store.
//!
//! The failure is a real one and deterministic: a trigger of the test makes every insert of an item of one job raise, which
//! is the statement a round makes after it has read its page, so the round fails inside its transaction and rolls it back, as
//! a statement over its timeout or a deadlock victim does. The trigger is dropped to let the job recover. No sleep stands in
//! for an event: the backoff is moved by the owner (`the_backoff_ends`), as the slots of a job are in `batch_pass`, and every
//! wait is a bounded wait for a state, with a message that says what it was waiting for.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use sagat_application::batch::{BatchStore, DueJob, JobRecord, SelectionFailure};
use sagat_application::batch_executor::{BatchExecutor, ExecutorSettings, Stepped};
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::batch::{
    Action, FailureReason, JobState, Retry, SELECTION_RETRIES, Selector, selection_retry,
};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::batch_executor::{
    Runs, create, due, everything, job, live, runs_started, service, write,
};
use crate::counted::{recorded, series};
use crate::firing::{Fixture, fixture};
use crate::support::schedules::TestClock;

type Executor = BatchExecutor<PgBatchStore, Runs, PgIdentityStore, TestClock>;

/// How long a wait for a state of the database may take: the selection of a job waits for the oldest transaction open on the
/// whole server, and the tests of this suite share one.
const GIVE_UP_AFTER: StdDuration = StdDuration::from_secs(180);

/// The pool the executor of a test connects with: the owner, or `sagat_app` when the suite runs as the runtime role, so that
/// every statement the executor makes for a failed round is made by the role that makes it in production.
pub(crate) async fn role_pool(f: &Fixture) -> PgPool {
    if crate::support::as_runtime_role() {
        f.w.db.runtime_pool_with(4).await
    } else {
        f.w.db.pool.clone()
    }
}

pub(crate) fn executor_on(f: &Fixture, pool: &PgPool, settings: ExecutorSettings) -> Executor {
    BatchExecutor::new(
        PgBatchStore::new(pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(pool.clone()),
        f.clock.clone(),
        settings,
    )
}

/// Makes every insert of an item of `job` fail, as the statement of a round that cannot succeed does. Returns the name of the
/// trigger. The function is the test's own and the text it raises names nothing of the job.
pub(crate) async fn break_selection_on(owner: &PgPool, job: Uuid) -> String {
    break_selection_quoting(
        owner,
        job,
        "the selection of this job is made to fail by the test",
    )
    .await
}

/// As [`break_selection_on`], with the message the database error carries: the message of a database error can quote a value, and
/// this one quotes whatever the test gives it (SQLSTATE `P0001`).
pub(crate) async fn break_selection_quoting(owner: &PgPool, job: Uuid, message: &str) -> String {
    let name = format!("selection_probe_{}", job.simple());
    // the name is a prefix of ours and a UUID, the message is the test's own text with its quotes and `%` made literal
    let message = message.replace('\'', "''").replace('%', "%%");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE OR REPLACE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION '{message}' USING ERRCODE = 'P0001'; END $$"
    )))
    .execute(owner)
    .await
    .expect("the function that fails");
    // `job` is a UUID, never user input
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER {name} BEFORE INSERT ON batch_items FOR EACH ROW \
         WHEN (NEW.job_id = '{job}') EXECUTE FUNCTION {name}()"
    )))
    .execute(owner)
    .await
    .expect("the trigger that fails the selection");
    name
}

pub(crate) async fn break_selection(f: &Fixture, job: Uuid) -> String {
    break_selection_on(&f.w.db.pool, job).await
}

/// Lets the selection of the job work again.
pub(crate) async fn heal_selection(f: &Fixture, job: Uuid) {
    // the name is a prefix of ours and a UUID
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER selection_probe_{} ON batch_items",
        job.simple()
    )))
    .execute(&f.w.db.pool)
    .await
    .expect("the selection works again");
}

/// The backoff of the job is over: its slot is now, by the clock of the database.
pub(crate) async fn the_backoff_ends_on(owner: &PgPool, job: Uuid) {
    sqlx::query("UPDATE batch_jobs SET next_slot_at = clock_timestamp() WHERE id = $1")
        .bind(job)
        .execute(owner)
        .await
        .expect("the slot comes");
}

pub(crate) async fn the_backoff_ends(f: &Fixture, job: Uuid) {
    the_backoff_ends_on(&f.w.db.pool, job).await;
}

/// The job waits an hour, so that a check made after the record does not depend on how long the machine took to get to it.
pub(crate) async fn the_backoff_is_long(f: &Fixture, job: Uuid) {
    sqlx::query(
        "UPDATE batch_jobs SET next_slot_at = clock_timestamp() + interval '1 hour' WHERE id = $1",
    )
    .bind(job)
    .execute(&f.w.db.pool)
    .await
    .expect("a long wait");
}

async fn db_now(f: &Fixture) -> OffsetDateTime {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the clock of the database")
}

/// What the table says of a job: state, failures in a row, reason, slot, selected, cursor, end.
#[derive(Debug, PartialEq)]
pub(crate) struct Stored {
    pub(crate) state: String,
    pub(crate) failures: i16,
    pub(crate) reason: Option<String>,
    pub(crate) slot: OffsetDateTime,
    pub(crate) selected: i32,
    pub(crate) cursor: Option<Uuid>,
    pub(crate) finished: Option<OffsetDateTime>,
}

pub(crate) async fn stored(f: &Fixture, id: Uuid) -> Stored {
    let row: (
        String,
        i16,
        Option<String>,
        OffsetDateTime,
        i32,
        Option<Uuid>,
        Option<OffsetDateTime>,
    ) = sqlx::query_as(
        "SELECT state, selection_failures, failure_reason, next_slot_at, selected, select_cursor, finished_at \
         FROM batch_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the job row");
    Stored {
        state: row.0,
        failures: row.1,
        reason: row.2,
        slot: row.3,
        selected: row.4,
        cursor: row.5,
        finished: row.6,
    }
}

/// `(state, reason)` of every audit record of the end of the job.
pub(crate) async fn endings(f: &Fixture, id: Uuid) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'batch.finished' AND resource_id = $1 ORDER BY created_at",
    )
    .bind(id.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit")
}

async fn item_states(f: &Fixture, id: Uuid) -> Vec<(i32, String)> {
    sqlx::query_as("SELECT ordinal, state FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("items")
}

/// Steps until the step does something other than wait. A step that waits is the selection that waits for a transaction open
/// since the creation of the job (the horizon is the whole server's), or a backoff that has not passed: the caller says which
/// by moving the backoff first. Bounded by a time, and the message says what was waited for.
pub(crate) async fn until_it_acts(
    exec: &Executor,
    job: &DueJob,
    fence: Option<&ShardFence>,
    what: &str,
) -> Stepped {
    let give_up = Instant::now() + GIVE_UP_AFTER;
    loop {
        let stepped = exec.step(job, fence.cloned()).await.expect("step");
        if stepped != Stepped::Idle {
            return stepped;
        }
        assert!(
            Instant::now() < give_up,
            "{what}: the job {} stayed idle for {GIVE_UP_AFTER:?}: its selection waits for a transaction of the whole server, or for a backoff nothing moved",
            job.id
        );
        tokio::time::sleep(StdDuration::from_millis(20)).await;
    }
}

fn engine_audit(job: &JobRecord) -> AuditRecord {
    let engine = Principal::new(job.tenant_id, Subject::engine(), Role::Operator);
    AuditRecord::by(
        &engine,
        AuditAction::BatchFinished,
        ResourceType::BatchJob,
        job.id.to_string(),
        None,
        OffsetDateTime::now_utc(),
    )
    .with_reason(FailureReason::SelectionFailed.as_str())
}

fn fence(owner: &str, shard: Shard, generation: u64) -> ShardFence {
    ShardFence {
        owner: owner.to_owned(),
        lease: Lease { shard, generation },
    }
}

/// `owner` holds the shard of `job` at `generation` for an hour.
async fn hold_the_shard(f: &Fixture, job: &JobRecord, owner: &str, generation: i64) {
    sqlx::query(
        "UPDATE scheduler_shards SET owner = $2, generation = $3, expires_at = clock_timestamp() + interval '1 hour' \
         WHERE shard = $1",
    )
    .bind(i16::from(Shard::of(job.id).index()))
    .bind(owner)
    .bind(generation)
    .execute(&f.w.db.pool)
    .await
    .expect("the shard is held");
}

// --- the bound ---------------------------------------------------------------------------------------------------------

/// Ten failed rounds in a row are survived, each followed by a wait that is the one of a refused item (5 s doubling to 5 min,
/// by the database's clock), and the eleventh ends the job `FAILED` with `SELECTION_FAILED`, with the audit record that
/// `AUTHORITY_REVOKED` writes. Catches: a bound off by one (the job fails at the tenth, or still selects after the eleventh),
/// a count that is not stored or is reset by a failure (it never reaches ten), a backoff that is not written or is read from
/// the wrong clock, a failure that is not audited once, a metric that is not counted once committed.
#[test]
fn a_selection_that_always_fails_waits_after_each_round_and_fails_the_job_at_the_bound() {
    let page = recorded(|| async {
        let f = fixture().await;
        runs_started(&f, 3).await;
        live(&f);
        let pool = role_pool(&f).await;
        let exec = executor_on(&f, &pool, ExecutorSettings::default());
        let created = create(&f, write(Action::Cancel, everything()), "selection-bound").await;
        break_selection(&f, created.id).await;
        let record = due(&created);

        for failures in 1..=SELECTION_RETRIES {
            let before = db_now(&f).await;
            let stepped = until_it_acts(&exec, &record, None, "a failing round").await;
            let after = db_now(&f).await;
            assert_eq!(
                stepped,
                Stepped::Backoff,
                "round {failures}: a failed round is recorded and the job waits"
            );
            let now = stored(&f, created.id).await;
            assert_eq!(
                (now.state.as_str(), i32::from(now.failures)),
                ("SELECTING", i32::try_from(failures).expect("count")),
                "round {failures}: still selecting, one more failure on record"
            );
            assert_eq!(
                (
                    now.selected,
                    now.cursor,
                    item_states(&f, created.id).await.len()
                ),
                (0, None, 0),
                "round {failures}: the round rolled back its own work"
            );
            let Retry::After(wait) = selection_retry(failures) else {
                panic!("round {failures} is within the bound");
            };
            assert!(
                now.slot >= before + wait && now.slot <= after + wait,
                "round {failures}: the next round is {wait} after the failure by the database's clock: \
                 slot {}, recorded between {before} and {after}",
                now.slot
            );
            // before its time the job is not stepped, and nothing moves (the wait is made an hour long first, so that this
            // does not depend on how long the machine took since the record: the window above is what is asserted of it)
            the_backoff_is_long(&f, created.id).await;
            let waiting = stored(&f, created.id).await;
            assert_eq!(
                exec.step(&record, None).await.expect("step"),
                Stepped::Idle,
                "round {failures}: the job waits its backoff"
            );
            assert_eq!(
                stored(&f, created.id).await,
                waiting,
                "round {failures}: a step before the time of the job changes nothing"
            );
            the_backoff_ends(&f, created.id).await;
        }

        // ten were survived; the job is still selecting and has no end
        let waiting = stored(&f, created.id).await;
        assert_eq!(
            (
                waiting.state.as_str(),
                waiting.reason.clone(),
                waiting.finished
            ),
            ("SELECTING", None, None),
            "before the bound the job is selecting"
        );
        assert!(
            endings(&f, created.id).await.is_empty(),
            "and has no end on record"
        );

        let stepped = until_it_acts(&exec, &record, None, "the round past the bound").await;
        assert_eq!(stepped, Stepped::Acted, "the eleventh failure ends the job");
        let ended = stored(&f, created.id).await;
        assert_eq!(
            (
                ended.state.as_str(),
                ended.reason.as_deref(),
                ended.failures
            ),
            (
                "FAILED",
                Some("SELECTION_FAILED"),
                i16::try_from(SELECTION_RETRIES + 1).expect("count")
            ),
            "FAILED, with the reason, and the count of the rounds that failed kept as evidence"
        );
        assert!(ended.finished.is_some(), "it has an end");
        assert_eq!(
            endings(&f, created.id).await,
            vec![Some("SELECTION_FAILED".to_owned())],
            "one audit record of the end, with the reason, as `AUTHORITY_REVOKED` writes it"
        );
        let read = job(&f, created.id).await;
        assert_eq!(read.state, JobState::Failed);
        assert_eq!(read.failure_reason, Some(FailureReason::SelectionFailed));
        assert_eq!(read.counts.selected, 0, "nothing was selected");
        // a job that has ended is not stepped again, and says nothing more
        assert_eq!(exec.step(&record, None).await.expect("step"), Stepped::Idle);
        assert_eq!(endings(&f, created.id).await.len(), 1, "still one");
    });
    // one retry for each failed round that did not end the job, one `failed` for the one that did, and no other label
    let counted = |outcome| {
        series(
            &page,
            "batch_selection_failures_total",
            &[("outcome", outcome)],
        )
    };
    assert_eq!(
        counted("retry"),
        Some(f64::from(SELECTION_RETRIES)),
        "{page}"
    );
    assert_eq!(counted("failed"), Some(1.0), "{page}");
    for line in page
        .lines()
        .filter(|line| line.starts_with("batch_selection_failures_total"))
    {
        let labels = &line[line.find('{').map_or(0, |i| i + 1)..line.rfind('}').unwrap_or(0)];
        assert!(
            labels == "outcome=\"retry\"" || labels == "outcome=\"failed\"",
            "a label of the series is not one of the two closed words: `{line}`"
        );
    }
}

/// A failure that passes is not the end of the job: one failed round, then a round that picks. The count goes back to zero in
/// the transaction that picks, and the job selects as if nothing had happened. Catches: a count that a pick does not reset
/// (it stays at one on a job that runs), a failure that ends the job at once, and a round that is not put off by the backoff.
#[tokio::test]
async fn one_failed_round_does_not_fail_the_job_and_the_round_that_picks_starts_the_count_again() {
    let f = fixture().await;
    let runs = runs_started(&f, 3).await;
    live(&f);
    let pool = role_pool(&f).await;
    let exec = executor_on(&f, &pool, ExecutorSettings::default());
    let created = create(
        &f,
        write(Action::Cancel, everything()),
        "selection-transient",
    )
    .await;
    let record = due(&created);
    break_selection(&f, created.id).await;

    assert_eq!(
        until_it_acts(&exec, &record, None, "the transient failure").await,
        Stepped::Backoff
    );
    assert_eq!(stored(&f, created.id).await.failures, 1);

    heal_selection(&f, created.id).await;
    // the backoff is not forgiven because the database is well again: the job waits its time, and the round it makes then picks
    the_backoff_is_long(&f, created.id).await;
    assert_eq!(
        exec.step(&record, None).await.expect("step"),
        Stepped::Idle,
        "a job that waits does not select before its time, even when it could"
    );
    assert_eq!(stored(&f, created.id).await.selected, 0);
    the_backoff_ends(&f, created.id).await;
    assert_eq!(
        until_it_acts(&exec, &record, None, "the round that picks").await,
        Stepped::Acted,
        "the next round picks"
    );

    let running = stored(&f, created.id).await;
    assert_eq!(
        (running.state.as_str(), running.failures, running.selected),
        ("RUNNING", 0, 3),
        "the job runs, with all its runs, and the failures in a row are none"
    );
    let selected: Vec<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
            .bind(created.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items");
    assert_eq!(selected, runs, "its three runs, by id");
    assert!(endings(&f, created.id).await.is_empty(), "nothing ended");
}

/// The failures that count are the ones in a row: a round that picks a page of several starts them again, so a job that fails
/// three rounds, picks, and fails two more has two on record and not five, and when at last it fails for good it keeps the
/// runs it had selected as items, now cancelled, and touches none of them. Catches: a reset that a pick that does not finish the
/// selection does not make (the count would be five), and a failure that touches a run.
#[tokio::test]
async fn a_pick_that_does_not_finish_the_selection_starts_the_count_again_and_a_failed_job_keeps_what_it_selected()
 {
    let f = fixture().await;
    let runs = runs_started(&f, 5).await;
    live(&f);
    let pool = role_pool(&f).await;
    let exec = executor_on(
        &f,
        &pool,
        ExecutorSettings {
            select_page: 2,
            ..ExecutorSettings::default()
        },
    );
    let created = create(&f, write(Action::Cancel, everything()), "selection-pages").await;
    let record = due(&created);

    break_selection(&f, created.id).await;
    for failures in 1..=3 {
        assert_eq!(
            until_it_acts(&exec, &record, None, "a failing round").await,
            Stepped::Backoff
        );
        assert_eq!(i32::from(stored(&f, created.id).await.failures), failures);
        the_backoff_ends(&f, created.id).await;
    }
    heal_selection(&f, created.id).await;
    assert_eq!(
        until_it_acts(&exec, &record, None, "the first page").await,
        Stepped::Acted
    );
    let paged = stored(&f, created.id).await;
    assert_eq!(
        (paged.state.as_str(), paged.failures, paged.selected),
        ("SELECTING", 0, 2),
        "a page was picked and the selection goes on: the failures in a row are none"
    );

    // two more failures are two, not five; the job is not nearer its end for the three before the page
    break_selection(&f, created.id).await;
    for failures in 1..=2 {
        the_backoff_ends(&f, created.id).await;
        assert_eq!(
            until_it_acts(&exec, &record, None, "a failing round after the page").await,
            Stepped::Backoff
        );
        assert_eq!(i32::from(stored(&f, created.id).await.failures), failures);
    }
    // and the rest of the bound is the whole of it: nine more rounds are survived, the next ends the job
    for failures in 3..=SELECTION_RETRIES {
        the_backoff_ends(&f, created.id).await;
        assert_eq!(
            until_it_acts(&exec, &record, None, "a failing round after the page").await,
            Stepped::Backoff,
            "failure {failures} in a row since the page"
        );
    }
    assert_eq!(
        stored(&f, created.id).await.state,
        "SELECTING",
        "ten in a row since the page, and still selecting"
    );
    the_backoff_ends(&f, created.id).await;
    assert_eq!(
        until_it_acts(&exec, &record, None, "the failure past the bound").await,
        Stepped::Acted
    );

    let ended = stored(&f, created.id).await;
    assert_eq!(
        (
            ended.state.as_str(),
            ended.reason.as_deref(),
            ended.selected
        ),
        ("FAILED", Some("SELECTION_FAILED"), 2),
        "failed, and the two runs it had selected are still its own"
    );
    assert_eq!(
        item_states(&f, created.id).await,
        vec![(1, "CANCELLED".to_owned()), (2, "CANCELLED".to_owned())],
        "the items it had not applied are cancelled, as when `AUTHORITY_REVOKED` ends a job, so that the counters equal the items"
    );
    let read = job(&f, created.id).await;
    assert_eq!(
        (
            read.counts.selected,
            read.counts.cancelled,
            read.counts.succeeded
        ),
        (2, 2, 0)
    );
    for run in &runs {
        let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
            .bind(run)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("status");
        assert_ne!(
            status, "CANCELLED",
            "a job that failed to select touches no run"
        );
    }
}

// --- a restart and a move of the lease -----------------------------------------------------------------------------------------

/// The count is the database's, not the process's: a replica that counted four failures is replaced by another, with nothing
/// in its memory, which carries on from four and ends the job at the eleventh. A replica that has lost the lease is refused
/// where it records and the count does not move. Catches: a count held in the executor (the new one would start from nothing
/// and need eleven rounds of its own), and a record of a failure that does not check the fence of the shard.
#[tokio::test]
async fn the_count_survives_a_restart_and_a_move_of_the_lease_and_a_replica_that_lost_it_records_nothing()
 {
    let f = fixture().await;
    runs_started(&f, 2).await;
    live(&f);
    let pool = role_pool(&f).await;
    let created = create(&f, write(Action::Cancel, everything()), "selection-lease").await;
    let record = due(&created);
    let shard = Shard::of(created.id);
    break_selection(&f, created.id).await;

    // replica a holds the shard and counts four failed rounds
    hold_the_shard(&f, &created, "replica-a", 1).await;
    let a = executor_on(&f, &pool, ExecutorSettings::default());
    let held_by_a = fence("replica-a", shard, 1);
    for failures in 1..=4 {
        assert_eq!(
            until_it_acts(&a, &record, Some(&held_by_a), "replica a, a failing round").await,
            Stepped::Backoff
        );
        assert_eq!(i32::from(stored(&f, created.id).await.failures), failures);
        the_backoff_ends(&f, created.id).await;
    }

    // replica a dies; replica b takes the shard at the next generation
    hold_the_shard(&f, &created, "replica-b", 2).await;

    // a replica that has not noticed is refused where it records, and where it selects: nothing of the job moves
    let before = stored(&f, created.id).await;
    let refused = PgBatchStore::new(pool.clone())
        .selection_failed(
            created.tenant_id,
            created.id,
            OffsetDateTime::now_utc(),
            Some(&held_by_a),
            &engine_audit(&created),
        )
        .await;
    assert!(
        matches!(refused, Err(sagat_application::error::StoreError::StateConflict(reason)) if reason.contains("lease")),
        "the record of a failure under a lease that was lost is refused: {refused:?}"
    );
    assert!(
        matches!(
            a.step(&record, Some(held_by_a.clone()))
                .await
                .expect("step"),
            Stepped::Refused(_)
        ),
        "and so is the round of that replica"
    );
    assert_eq!(
        stored(&f, created.id).await,
        before,
        "the usurped replica changed nothing"
    );
    assert!(endings(&f, created.id).await.is_empty());

    // replica b, started from nothing but the database, carries on from four: seven more rounds end the job, not eleven
    let b = executor_on(&f, &pool, ExecutorSettings::default());
    let held_by_b = fence("replica-b", shard, 2);
    for failures in 5..=SELECTION_RETRIES {
        assert_eq!(
            until_it_acts(&b, &record, Some(&held_by_b), "replica b, a failing round").await,
            Stepped::Backoff,
            "replica b, failure {failures} in a row"
        );
        assert_eq!(
            i32::from(stored(&f, created.id).await.failures),
            i32::try_from(failures).expect("count"),
            "the count went on from the four that replica a left"
        );
        the_backoff_ends(&f, created.id).await;
    }
    assert_eq!(
        until_it_acts(
            &b,
            &record,
            Some(&held_by_b),
            "replica b, the failure past the bound"
        )
        .await,
        Stepped::Acted,
        "replica b ended the job at the eleventh failure, counted across two replicas"
    );
    let ended = stored(&f, created.id).await;
    assert_eq!(
        (ended.state.as_str(), ended.reason.as_deref()),
        ("FAILED", Some("SELECTION_FAILED"))
    );
    assert_eq!(
        endings(&f, created.id).await.len(),
        1,
        "one record of the end"
    );
}

// --- cancelling, and what the record of a failure does not touch ---------------------------------------------------------------

/// A job that waits its backoff is cancelled like any other and its cancel is not held back by the wait; a failure recorded for a
/// job that is no longer selecting records nothing (the round that failed was decided before the cancel). Catches: a record that
/// counts a failure of a cancelled job, and a cancel that the backoff delays.
#[tokio::test]
async fn a_job_that_waits_can_be_cancelled_and_a_failure_of_a_job_that_is_not_selecting_records_nothing()
 {
    let f = fixture().await;
    runs_started(&f, 2).await;
    live(&f);
    let pool = role_pool(&f).await;
    let exec = executor_on(&f, &pool, ExecutorSettings::default());
    let created = create(&f, write(Action::Cancel, everything()), "selection-cancel").await;
    let record = due(&created);
    break_selection(&f, created.id).await;
    for _ in 0..3 {
        assert_eq!(
            until_it_acts(&exec, &record, None, "a failing round").await,
            Stepped::Backoff
        );
        the_backoff_ends(&f, created.id).await;
    }
    assert_eq!(
        until_it_acts(&exec, &record, None, "the fourth failing round").await,
        Stepped::Backoff
    );
    let waiting = stored(&f, created.id).await;
    assert_eq!(waiting.failures, 4);

    service(&f)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job",
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("cancel the waiting job");
    let cancelling = stored(&f, created.id).await;
    assert_eq!(cancelling.state, "CANCELLING");
    assert!(
        cancelling.slot < waiting.slot,
        "the cancel brings the slot of the job forward: the wait does not hold it back ({} against {})",
        cancelling.slot,
        waiting.slot
    );

    // a failure that arrives for a job that is cancelling records nothing and says so
    let before = stored(&f, created.id).await;
    let outcome = PgBatchStore::new(pool.clone())
        .selection_failed(
            created.tenant_id,
            created.id,
            OffsetDateTime::now_utc(),
            None,
            &engine_audit(&created),
        )
        .await
        .expect("the record");
    assert_eq!(outcome, SelectionFailure::NotSelecting);
    assert_eq!(stored(&f, created.id).await, before, "nothing was recorded");

    // and the cancel goes through: the count stays on the cancelled job, as the evidence of the rounds that failed
    assert_eq!(
        until_it_acts(&exec, &record, None, "the drain of the cancelled job").await,
        Stepped::Acted
    );
    let ended = stored(&f, created.id).await;
    assert_eq!((ended.state.as_str(), ended.failures), ("CANCELLED", 4));
    assert_eq!(
        endings(&f, created.id).await,
        vec![Some("CANCELLED".to_owned())],
        "one end, the cancel's"
    );
}

/// What a selection that fails costs the others: a job that backs off leaves the scan until its time, so the job behind it is
/// reached even when the scan takes one job a round (a job that stayed due would be the head of every round, for ever); the scan
/// never returns the job before its time by the database's clock, returns it at its time and after, and a round that has only it
/// to wait for does not ask the loop to come back at once. Catches: a record that writes no wait (the slot written is asserted to
/// be ahead of the round by the database's clock, and the job would stay the head of the scan), a failed round that is left due
/// (the healthy job starves, which this test reports as a wait that ends), a scan that reads the wait from another clock, and a
/// hint that makes the loop spin. After the assertion on the wait it makes the wait an hour long, so that what follows does not
/// depend on the machine's speed.
#[tokio::test]
async fn a_job_that_waits_leaves_the_scan_until_its_time_and_the_jobs_behind_it_are_reached() {
    use sagat_application::batch::BatchService;
    use sagat_application::clock::SystemClock;
    use sagat_application::identity::RequestContext;
    use sagat_infra_postgres::DatabaseClock;
    use sagat_scheduler::{BatchPass, BatchSettings, Keeper, KeeperSettings, Placement};

    let f = fixture().await;
    let runs = runs_started(&f, 2).await;
    let pool = role_pool(&f).await;
    let clock = DatabaseClock::new(f.w.db.pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");
    let placement = Placement::new(format!("scan-{}", Uuid::now_v7()));
    Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("the keeper holds the shards");
    // one job a round: a job that stays due at the head of the scan is the only one ever looked at
    let pass = BatchPass::new(
        BatchExecutor::new(
            PgBatchStore::new(pool.clone()),
            Runs(Arc::clone(&f.w)),
            PgIdentityStore::new(pool.clone()),
            clock.clone(),
            ExecutorSettings::default(),
        ),
        placement,
        BatchSettings {
            batch: 1,
            ..BatchSettings::default()
        },
    );
    let store = PgBatchStore::new(pool.clone());
    let create = |selector: Selector, key: &'static str| {
        let (f, clock) = (&f, clock.clone());
        async move {
            BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), clock)
                .create(
                    &f.w.admin,
                    f.w.namespace.id(),
                    write(Action::Cancel, selector),
                    key,
                    &RequestContext::default(),
                )
                .await
                .expect("create the job")
        }
    };

    // the failing job is the older: its slot is the first, so it is the head of the scan; the healthy one names a run
    let failing = create(everything(), "scan-failing").await;
    let healthy = create(
        Selector::run_ids(vec![runs[0]]).expect("selector"),
        "scan-healthy",
    )
    .await;
    break_selection(&f, failing.id).await;

    let give_up = Instant::now() + GIVE_UP_AFTER;
    let mut recorded_failure = false;
    while stored(&f, healthy.id).await.state != "COMPLETED" {
        let before = db_now(&f).await;
        let round = pass.round().await;
        if !recorded_failure && stored(&f, failing.id).await.failures == 1 {
            recorded_failure = true;
            // the wait that was written is ahead of the round by the database's clock, whatever the machine's speed: this is what
            // takes the job out of the scan, and what a record that writes no wait does not do
            let Retry::After(wait) = selection_retry(1) else {
                panic!("the first failure is within the bound");
            };
            let written = stored(&f, failing.id).await.slot;
            assert!(
                written >= before + wait,
                "the first failed round left no wait on the job: slot {written}, round started at {before}, wait {wait}"
            );
            assert_eq!(
                (round.steps, round.failed),
                (0, 1),
                "the round that met the failing job recorded its failure and took no step"
            );
            // its wait is as long as the test needs: the real wait is asserted in `a_selection_that_always_fails_...`
            the_backoff_is_long(&f, failing.id).await;
        }
        assert!(
            Instant::now() < give_up,
            "the healthy job did not complete in {GIVE_UP_AFTER:?} (the failure of the other was recorded: {recorded_failure}): \
             a job that waits keeps the head of the scan"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    assert!(
        recorded_failure,
        "the failing job was met before the healthy one"
    );
    assert_eq!(
        stored(&f, failing.id).await.failures,
        1,
        "the failing job was not looked at again while it waits"
    );
    assert_eq!(stored(&f, failing.id).await.state, "SELECTING");

    // the scan, decided at an instant: not due a millisecond before its slot, due at it, and never lost after it
    let slot = stored(&f, failing.id).await.slot;
    let shard = [Shard::of(failing.id)];
    let ms = time::Duration::milliseconds(1);
    let found = |at: OffsetDateTime| {
        let (store, shard) = (&store, &shard);
        async move {
            store
                .due_jobs_as_of(shard, 10, Some(at))
                .await
                .expect("the scan")
                .jobs
                .into_iter()
                .any(|due| due.id == failing.id)
        }
    };
    assert!(!found(slot - ms).await, "not before its time");
    assert!(found(slot).await, "at its time");
    assert!(found(slot + ms).await, "and after it: never lost");

    // nothing is due while it waits: a round does nothing and does not ask the loop to look again in a few milliseconds
    let idle = pass.round().await;
    assert_eq!(
        (idle.due, idle.steps, idle.refused, idle.failed),
        (0, 0, 0, 0),
        "a job that waits is not scanned, nor stepped"
    );
    assert!(
        idle.next_due_in
            .is_none_or(|soon| soon >= StdDuration::from_secs(1)),
        "a job that waits does not make the loop look again at once: {:?}",
        idle.next_due_in
    );
}
// --- a cancel that commits while a failure is recorded -----------------------------------------------------------------------

/// Waits, by `pg_stat_activity` and with no sleep standing in for the event, until a backend waits on a lock while running a
/// statement about batch jobs: the record of a failure that has reached the row of the job a transaction of the test holds.
async fn until_a_statement_waits_for_the_job(f: &Fixture) {
    crate::support::bounded::within(
        "the record of a failure waits for the row of the job",
        StdDuration::from_secs(120),
        async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid() \
                       AND wait_event_type = 'Lock' AND query LIKE '%batch_jobs%'",
                )
                .fetch_one(&f.w.db.pool)
                .await
                .expect("look at the backends");
                if waiting >= 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
}

/// The row of the job is what protects a cancel from the record of a failure (D2204): the record of a failure reads the job `FOR
/// UPDATE`, so a cancel that commits while it waits is seen, and the record changes nothing. The cancel's transaction (its own
/// write, as `request_cancel` makes it, uncommitted) holds the row; the record starts, and the test waits until the database says
/// it waits for that row; the cancel commits. Nothing moves: not the count, not the slot the cancel pulled forward, and, with
/// ten failures on record, the job is not ended `FAILED` over its `CANCELLING` (the guard trigger of the table allows that
/// transition, so the lock is the only protection). Catches: a record that reads the job without its row lock (the write that
/// follows waits for the cancel, then overwrites it).
#[tokio::test]
async fn a_cancel_that_commits_while_a_failure_is_recorded_is_not_overwritten_and_the_failure_records_nothing()
 {
    let f = fixture().await;
    runs_started(&f, 1).await;
    live(&f);
    let pool = role_pool(&f).await;
    let store = PgBatchStore::new(pool.clone());
    for (case, was) in [
        ("with failures on record, short of the bound", 3_i16),
        (
            "at the bound, where the record would end the job",
            i16::try_from(SELECTION_RETRIES).expect("count"),
        ),
    ] {
        let created = create(
            &f,
            write(Action::Cancel, everything()),
            &format!("selection-race-{was}"),
        )
        .await;
        sqlx::query("UPDATE batch_jobs SET selection_failures = $2 WHERE id = $1")
            .bind(created.id)
            .bind(was)
            .execute(&f.w.db.pool)
            .await
            .expect("the failures on record");
        // the cancel: its write, held uncommitted
        let mut cancel =
            f.w.db
                .pool
                .begin()
                .await
                .expect("the transaction of the cancel");
        let cancel_slot: OffsetDateTime = sqlx::query_scalar(
            "UPDATE batch_jobs SET state = 'CANCELLING', next_slot_at = LEAST(next_slot_at, clock_timestamp()) \
             WHERE id = $1 RETURNING next_slot_at",
        )
        .bind(created.id)
        .fetch_one(&mut *cancel)
        .await
        .expect("the write of the cancel");
        // the record of a failure starts: it reaches the row and waits for it
        let record = tokio::spawn({
            let store = store.clone();
            let audit = engine_audit(&created);
            let (tenant, id) = (created.tenant_id, created.id);
            async move {
                store
                    .selection_failed(tenant, id, OffsetDateTime::now_utc(), None, &audit)
                    .await
            }
        });
        until_a_statement_waits_for_the_job(&f).await;
        cancel.commit().await.expect("the cancel commits");

        let outcome = record.await.expect("join").expect("the record");
        assert_eq!(
            outcome,
            SelectionFailure::NotSelecting,
            "{case}: the cancel that committed while the record waited is seen"
        );
        let after = stored(&f, created.id).await;
        assert_eq!(
            (after.state.as_str(), after.failures),
            ("CANCELLING", was),
            "{case}: the job is still cancelling and no failure was counted"
        );
        assert_eq!(
            after.slot, cancel_slot,
            "{case}: the slot is the one the cancel pulled forward"
        );
        assert!(
            after.reason.is_none()
                && after.finished.is_none()
                && endings(&f, created.id).await.is_empty(),
            "{case}: the job was not ended under the cancel: {after:?}"
        );
    }
}

// --- the cancel of a job that waits is due at once -----------------------------------------------------------------------------

/// The cancel pulls the slot of the job back to the database's now, not to the service's: a job that waits its backoff has a slot
/// ahead (a minute or five), and a service whose clock runs ahead of the database's would have left the cancel waiting for the
/// difference. The clock of the service is a minute ahead here, and so is the slot of a job that waits an hour: the cancel is due by
/// the database's clock at once, and the scan returns it. The same holds for a running job whose next item is a thousand seconds
/// away (the pull-back is the database's for every job). Catches: a pull-back by the clock of the service (the slot would be a
/// minute ahead of the database).
#[tokio::test]
async fn the_cancel_of_a_job_that_waits_is_due_at_once_whatever_the_clock_of_the_service_says() {
    use sagat_domain::batch::Rate;

    let f = fixture().await;
    runs_started(&f, 2).await;
    let pool = role_pool(&f).await;
    let exec = executor_on(&f, &pool, ExecutorSettings::default());
    let store = PgBatchStore::new(pool.clone());
    // the clock of the service, and of the executor, runs a minute ahead of the database's
    f.clock
        .set(OffsetDateTime::now_utc() + time::Duration::minutes(1));
    let cancel = |id: Uuid| {
        let f = &f;
        async move {
            service(f)
                .cancel(
                    &f.w.admin,
                    id,
                    "stop this batch job",
                    &sagat_application::identity::RequestContext::default(),
                )
                .await
                .expect("cancel the job");
        }
    };

    // a job that waits after a failed round
    let waiting = create(
        &f,
        write(Action::Cancel, everything()),
        "selection-cancel-clock-waiting",
    )
    .await;
    break_selection(&f, waiting.id).await;
    assert_eq!(
        until_it_acts(&exec, &due(&waiting), None, "a failing round").await,
        Stepped::Backoff
    );
    the_backoff_is_long(&f, waiting.id).await;
    cancel(waiting.id).await;
    let (slot, now) = (stored(&f, waiting.id).await.slot, db_now(&f).await);
    assert!(
        slot <= now,
        "the cancel of a job that waits is due by the database's clock at once: slot {slot}, database now {now}"
    );
    let shard = [Shard::of(waiting.id)];
    let scan = store.due_jobs(&shard, 10).await.expect("the scan");
    assert!(
        scan.jobs.iter().any(|due| due.id == waiting.id),
        "and the scan returns it"
    );

    // a running job whose next item is far ahead
    let running = create(
        &f,
        sagat_application::batch::BatchWrite {
            rate: Some(Rate::from_milli(1).expect("the slowest rate")),
            ..write(Action::Cancel, everything())
        },
        "selection-cancel-clock-running",
    )
    .await;
    assert_eq!(
        until_it_acts(&exec, &due(&running), None, "the selection").await,
        Stepped::Acted
    );
    assert_eq!(
        until_it_acts(&exec, &due(&running), None, "the first item").await,
        Stepped::Acted
    );
    assert!(
        stored(&f, running.id).await.slot > db_now(&f).await + time::Duration::seconds(300),
        "the next item of the running job is far ahead"
    );
    cancel(running.id).await;
    let (slot, now) = (stored(&f, running.id).await.slot, db_now(&f).await);
    assert!(
        slot <= now,
        "the cancel of a running job is due by the database's clock at once: slot {slot}, database now {now}"
    );
}

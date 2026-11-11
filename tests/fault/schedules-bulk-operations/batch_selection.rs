//! A selection round that fails, recorded under failure (`T018-046`, `G2093`, `D2197`, `FR-018-029`): a process that dies at any
//! write of the record of a failed round commits nothing of it, and the next step records it exactly once. The count is never lost
//! (a restart adds one, not none or two) and a job is never ended twice (one audit record, the items cancelled once).
//!
//! Two stages, each cut at every writing statement of its transaction as the matrix of `batch` does: the record of a failure
//! that the job survives (the count and the wait, one write), and the record of the failure past the bound, which ends the job
//! `SELECTION_FAILED` (the items cancelled, the job, its audit record). The round that failed is the real statement of the real
//! store that fails inside its transaction, because a trigger of the test makes every insert of an item raise: it writes
//! nothing, so the writes the switch counts are the record's alone.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use sagat_application::batch::{DueJob, JobRecord};
use sagat_application::batch_executor::{BatchExecutor, ExecutorSettings, Stepped};
use sagat_domain::batch::{FailureReason, JobState, SELECTION_RETRIES};
use sagat_domain::identity::{Principal, Role};
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::batch::{Applies, create, runs_started, service, until_it_acts};
use crate::support::schedules::{Fixture, TestClock, fixture};
use crate::tripwire::{Tripwire, table_digests};

type Executor = BatchExecutor<PgBatchStore, Applies, PgIdentityStore, TestClock>;

/// A process that starts from nothing but the database and the action, with a page of two runs for the selection.
fn executor_of(f: &Fixture, applies: &Applies) -> Executor {
    BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        applies.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings {
            select_page: 2,
            ..ExecutorSettings::default()
        },
    )
}

fn live(f: &Fixture) {
    f.clock
        .set(OffsetDateTime::now_utc() + Duration::seconds(2));
}

fn due(job: &JobRecord) -> DueJob {
    DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    }
}

async fn job(f: &Fixture, id: Uuid) -> JobRecord {
    service(f).get(&f.w.admin, id).await.expect("job")
}

/// What the table says of the job and nothing the service returns: the count, and the slot the next round is not before.
async fn failures_of(f: &Fixture, id: Uuid) -> (i16, OffsetDateTime) {
    sqlx::query_as("SELECT selection_failures, next_slot_at FROM batch_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the count")
}

async fn items(f: &Fixture, id: Uuid) -> Vec<(i32, String)> {
    sqlx::query_as("SELECT ordinal, state FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("items")
}

async fn endings_of(f: &Fixture, id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'batch.finished' AND resource_id = $1",
    )
    .bind(id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit")
}

/// Room for `jobs` jobs in progress at once: a job that waits for its next round is in progress, and the matrix leaves one of
/// them for each cut.
async fn room_for(f: &Fixture, jobs: i64) {
    for resource in ["batch_jobs", "tenant_batch_jobs"] {
        sqlx::query(
            "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
             VALUES ($1, $3, $2, 'operator')",
        )
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(jobs)
        .bind(resource)
        .execute(&f.w.db.pool)
        .await
        .expect("a higher ceiling");
    }
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', $3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(jobs)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_batch_jobs', $2, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(jobs)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs of the tenant");
}

/// Every insert of an item of the job fails, as the statement of a round that cannot succeed does.
async fn break_selection(f: &Fixture, job: Uuid) {
    sqlx::query(
        "CREATE OR REPLACE FUNCTION selection_probe_fails() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'the selection of this job is made to fail by the test' USING ERRCODE = 'P0001'; END $$",
    )
    .execute(&f.w.db.pool)
    .await
    .expect("the function that fails");
    // the name and the id are a prefix of ours and a UUID, never user input
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER selection_probe_{} BEFORE INSERT ON batch_items FOR EACH ROW \
         WHEN (NEW.job_id = '{job}') EXECUTE FUNCTION selection_probe_fails()",
        job.simple()
    )))
    .execute(&f.w.db.pool)
    .await
    .expect("the trigger that fails the selection");
}

#[derive(Debug, Clone, Copy)]
enum Stage {
    /// The record of a failure the job survives: the count and the wait.
    Survived,
    /// The record of the failure past the bound: the job ends `SELECTION_FAILED`, its pending items cancelled, its audit record.
    PastTheBound,
}

impl Stage {
    const fn name(self) -> &'static str {
        match self {
            Self::Survived => "a failure the job survives",
            Self::PastTheBound => "the failure past the bound",
        }
    }

    /// The count the job has before the record that is cut off.
    const fn before(self) -> i16 {
        match self {
            Self::Survived => 0,
            Self::PastTheBound => 10,
        }
    }

    /// A job one failed round short of the stage: the next step is the record that is cut off. Three runs and a page of two,
    /// so that the job has selected a page (two items waiting) before its selection starts to fail.
    async fn prepare(
        self,
        f: &Fixture,
        creator: &Principal,
        creator_runs: &[Uuid],
        name: &str,
    ) -> JobRecord {
        live(f);
        let created = create(f, creator, creator_runs, &format!("fault-selection-{name}")).await;
        let setup = executor_of(f, &Applies::default());
        let page = until_it_acts(f, &setup, &created)
            .await
            .expect("the first page");
        assert_eq!(page, Stepped::Acted, "{}: the first page", self.name());
        break_selection(f, created.id).await;
        if self.before() > 0 {
            // the failures that came before, as the rounds of a job that has been failing for a while leave them
            sqlx::query("UPDATE batch_jobs SET selection_failures = $2 WHERE id = $1")
                .bind(created.id)
                .bind(self.before())
                .execute(&f.w.db.pool)
                .await
                .expect("the count of the failures before");
        }
        created
    }

    /// What has to be true of a job after the record of the stage was cut off once and the next step made it.
    async fn assert_recorded(self, f: &Fixture, applies: &Applies, id: Uuid, runs: &[Uuid]) {
        let all = format!("{}, cut off", self.name());
        let (count, slot) = failures_of(f, id).await;
        assert_eq!(
            count,
            self.before() + 1,
            "{all}: the failure is on record once, not lost and not twice"
        );
        let read = job(f, id).await;
        match self {
            Self::Survived => {
                assert_eq!(read.state, JobState::Selecting, "{all}: the job goes on");
                assert_eq!(read.failure_reason, None, "{all}");
                assert_eq!(endings_of(f, id).await, 0, "{all}: no end");
                let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
                    .fetch_one(&f.w.db.pool)
                    .await
                    .expect("the clock of the database");
                assert!(
                    slot > now,
                    "{all}: the job waits its backoff: {slot} against {now}"
                );
                assert_eq!(
                    items(f, id).await,
                    vec![(1, "PENDING".to_owned()), (2, "PENDING".to_owned())],
                    "{all}: the page it had selected is untouched"
                );
            }
            Self::PastTheBound => {
                assert_eq!(read.state, JobState::Failed, "{all}");
                assert_eq!(
                    read.failure_reason,
                    Some(FailureReason::SelectionFailed),
                    "{all}"
                );
                assert_eq!(endings_of(f, id).await, 1, "{all}: one record of the end");
                assert_eq!(
                    items(f, id).await,
                    vec![(1, "CANCELLED".to_owned()), (2, "CANCELLED".to_owned())],
                    "{all}: the items that had not ended are cancelled, once"
                );
                assert_eq!(
                    (
                        read.counts.selected,
                        read.counts.cancelled,
                        read.counts.succeeded
                    ),
                    (2, 2, 0),
                    "{all}: the counters equal the items"
                );
            }
        }
        // nothing was applied to a run, whatever the stage and however often the record was tried
        let keys: BTreeSet<String> = applies
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(&format!("batch:{id}:")))
            .collect();
        assert!(
            keys.is_empty(),
            "{all}: no effect on a run: {keys:?} of {runs:?}"
        );
    }
}

async fn die_at_every_write(stage: Stage) {
    let f = fixture().await;
    room_for(&f, 16).await;
    let wire = Tripwire::install(&f.w.db.pool).await;
    let creator = f.w.member("batch-creator", Role::Operator).await;
    let runs = runs_started(&f, 3).await;
    let applies = Applies::default();
    let exec = executor_of(&f, &applies);

    // Which writes does the record make? Watch a normal one.
    let probe = stage
        .prepare(&f, &creator, &runs, &format!("{}-probe", stage.name()))
        .await;
    wire.watch().await;
    let recorded = until_it_acts(&f, &exec, &probe).await.expect("probe");
    wire.disarm().await;
    let want = match stage {
        Stage::Survived => Stepped::Backoff,
        Stage::PastTheBound => Stepped::Acted,
    };
    assert_eq!(recorded, want, "{}: the probe", stage.name());
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "{}: the record is one transaction and the round that failed wrote nothing: {writes:?}",
        stage.name()
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    assert!(
        tables.contains("batch_jobs"),
        "{}: the record writes batch_jobs: {writes:?}",
        stage.name()
    );
    if matches!(stage, Stage::PastTheBound) {
        assert!(
            tables.contains("audit_records") && tables.contains("batch_items"),
            "{}: the end of a job cancels its items and is audited in the same transaction: {writes:?}",
            stage.name()
        );
    } else {
        // the count and the wait are one `UPDATE` of the job, and the quota accounting of its row (a trigger) is the other
        // write the switch counts; nothing else is written
        assert_eq!(
            tables,
            BTreeSet::from(["batch_jobs", "quota_transaction_deltas"]),
            "{}: the count and the wait are one write of the job: {writes:?}",
            stage.name()
        );
    }
    println!(
        "{}: the record makes {} writing statements in one transaction, on {} tables: {tables:?}",
        stage.name(),
        writes.len(),
        tables.len()
    );
    stage.assert_recorded(&f, &applies, probe.id, &runs).await;

    // Die after each of those writes, on a job of its own, and start again.
    for (_, position, table) in &writes {
        let name = format!("{}-dies-at-{position}", stage.name());
        let applies = Applies::default();
        let exec = executor_of(&f, &applies);
        let job = stage.prepare(&f, &creator, &runs, &name).await;
        let digests = table_digests(&f.w.db.pool).await;
        let before = (self::job(&f, job.id).await, failures_of(&f, job.id).await);
        let items_before = items(&f, job.id).await;

        wire.arm(*position).await;
        let result = until_it_acts(&f, &exec, &job).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "{}: cut off at write {position} ({table}), the record answered {result:?}",
            stage.name()
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "{}: cut off at write {position} ({table}), a table changed",
            stage.name()
        );
        let after = (self::job(&f, job.id).await, failures_of(&f, job.id).await);
        assert_eq!(
            after,
            before,
            "{}: cut off at write {position} ({table}) moved the job on or lost a count",
            stage.name()
        );
        assert_eq!(items(&f, job.id).await, items_before);

        // The process restarts: a new executor, from nothing but the database. The round fails again and the failure is
        // recorded once.
        let applies_after = Applies::default();
        let restarted = executor_of(&f, &applies_after);
        let stepped = until_it_acts(&f, &restarted, &job)
            .await
            .expect("the restarted step");
        assert_eq!(stepped, want, "{}: after the restart", stage.name());
        let mut seen = applies.keys();
        seen.extend(applies_after.keys());
        stage
            .assert_recorded(&f, &Applies(Arc::new(Mutex::new(seen))), job.id, &runs)
            .await;
        // and it is recorded once: a second look finds the same count (the job that waits is not stepped, the one that ended
        // has nothing to do)
        let count = failures_of(&f, job.id).await.0;
        assert_eq!(
            until_it_acts_or_idle(&f, &restarted, &job).await,
            Stepped::Idle
        );
        assert_eq!(
            failures_of(&f, job.id).await.0,
            count,
            "{}: a step after the record changes no count",
            stage.name()
        );
    }

    // The matrix is the whole transaction: a record armed past its last write is not cut off.
    let beyond = stage
        .prepare(&f, &creator, &runs, &format!("{}-beyond", stage.name()))
        .await;
    wire.arm(writes.len() + 1).await;
    let result = until_it_acts(&f, &exec, &beyond).await;
    wire.disarm().await;
    assert_eq!(
        result.expect("not cut off"),
        want,
        "{}: the record makes more than the {} writes that were cut off",
        stage.name(),
        writes.len()
    );
}

/// One step, with no wait for a transaction: what a job that waits or has ended answers.
async fn until_it_acts_or_idle(f: &Fixture, exec: &Executor, job: &JobRecord) -> Stepped {
    f.clock.advance(Duration::seconds(1));
    exec.step(&due(job), None).await.expect("step")
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_record_of_a_failed_round_commits_nothing_and_the_failure_is_counted_once()
 {
    die_at_every_write(Stage::Survived).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_failure_past_the_bound_commits_nothing_and_the_job_fails_once()
 {
    assert_eq!(
        SELECTION_RETRIES,
        Stage::PastTheBound.before().unsigned_abs() as u32
    );
    die_at_every_write(Stage::PastTheBound).await;
}

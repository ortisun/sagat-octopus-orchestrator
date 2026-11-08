//! Batch jobs under failure (`T018-046`, `SC-018-009`, `FR-018-029..031`, `FR-018-033`): a process
//! that dies at any write of a step of the executor commits nothing of it, and the next step does it
//! exactly once; two steps at the same item count it once; a cancel that commits while an item is
//! being recorded loses neither the item nor the cancel.
//!
//! The matrix runs over the five transactions of the executor: a page of the selection, an item, the
//! last item (which also ends the job), the drain of a cancelled job and the failure of a job whose
//! creator lost authority. The action is one with no effect of its own that remembers the keys it
//! was asked for, so the only writes in a step are the executor's; the effect of the real run
//! service and its repetition under one key are proved in `batch_executor` of the integration suite.
//!
//! There is no lost hint to test: a job is found by the pass's own poll of the database and no broker
//! message announces it, so nothing a broker loses can delay or repeat a step.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use sagat_application::batch::{BatchService, BatchWrite, DueJob, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_domain::batch::{Action, JobState, Justification, Selector};
use sagat_domain::identity::{Principal, Role};
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::bounded::within;
use crate::support::schedules::{Fixture, TestClock, fixture};
use crate::tripwire::{Tripwire, table_digests};

/// An action with no effect of its own that remembers the keys it was asked for.
#[derive(Clone, Default)]
struct Applies(Arc<Mutex<Vec<String>>>);

impl Applies {
    fn keys(&self) -> Vec<String> {
        self.0.lock().expect("keys").clone()
    }
}

impl ItemActions for Applies {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        self.0.lock().expect("keys").push(request.key.to_owned());
        Ok(ItemEffect::Applied { new_run: None })
    }
}

type Executor = BatchExecutor<PgBatchStore, Applies, PgIdentityStore, TestClock>;

/// A process that starts from nothing but the database and the action: what a restarted one has.
fn executor(f: &Fixture, applies: &Applies) -> Executor {
    BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        applies.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings::default(),
    )
}

fn service(f: &Fixture) -> BatchService<PgBatchStore, TestClock> {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), f.clock.clone())
}

/// The clock of the test follows the database's time, which the jobs are stamped with, and then
/// moves by hand.
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

async fn runs_started(f: &Fixture, n: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for _ in 0..n {
        ids.push(f.w.start(f.workflow, &json!({})).await.as_uuid());
    }
    ids.sort_unstable();
    ids
}

async fn create(f: &Fixture, creator: &Principal, runs: &[Uuid], key: &str) -> JobRecord {
    service(f)
        .create(
            creator,
            f.w.namespace.id(),
            BatchWrite {
                selector: Selector::run_ids(runs.to_vec()).expect("selector"),
                action: Action::Cancel,
                step_id: None,
                instance_path: Vec::new(),
                signal: None,
                update: None,
                drop_business_id: false,
                repin: false,
                reason: Justification::new("clean up the runs that are stuck").expect("reason"),
                max_items: None,
                rate: None,
            },
            key,
            &RequestContext::default(),
        )
        .await
        .expect("create the job")
}

async fn job(f: &Fixture, id: Uuid) -> JobRecord {
    service(f).get(&f.w.admin, id).await.expect("job")
}

fn ended(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Cancelled | JobState::Failed
    )
}

/// `(ordinal, state)` of the items, in order.
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

/// Steps until the step does something other than wait. The job waits, as it should, for a
/// transaction of the time of creation that is still open (the world has loops of its own).
async fn until_it_acts(f: &Fixture, exec: &Executor, job: &JobRecord) -> Result<Stepped, AppError> {
    for _ in 0..1_000 {
        f.clock.advance(Duration::seconds(1));
        match exec.step(&due(job), None).await {
            Ok(Stepped::Idle) => tokio::time::sleep(StdDuration::from_millis(5)).await,
            other => return other,
        }
    }
    panic!("the job never acted");
}

/// Steps until the job has ended, and returns it.
async fn until_it_ends(f: &Fixture, exec: &Executor, job: &JobRecord) -> JobRecord {
    for _ in 0..1_000 {
        let now = self::job(f, job.id).await;
        if ended(now.state) {
            return now;
        }
        f.clock.advance(Duration::seconds(1));
        exec.step(&due(job), None).await.expect("step");
        tokio::time::sleep(StdDuration::from_millis(2)).await;
    }
    panic!("the job never ended");
}

// --- the matrix -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Stage {
    /// The page of the selection: the items and the cursor.
    Select,
    /// The first of three items: the outcome, the counters and the next slot.
    Item,
    /// The only item of a job, which also ends the job and writes the audit record of its end.
    LastItem,
    /// The drain of a job that was cancelled while it had pending items.
    Drain,
    /// The failure of a job whose creator lost a permission, with its pending items cancelled.
    Fail,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Select => "selection",
            Self::Item => "item",
            Self::LastItem => "last item",
            Self::Drain => "drain",
            Self::Fail => "failure",
        }
    }

    /// How many items the job has once it is selected.
    const fn items(self) -> usize {
        match self {
            Self::LastItem => 1,
            _ => 3,
        }
    }

    /// A job that is one step short of the stage: the next step is the one that is cut off.
    async fn prepare(
        self,
        f: &Fixture,
        exec: &Executor,
        creator: &Principal,
        runs: &[Uuid],
        name: &str,
    ) -> JobRecord {
        // the creator holds the role it is made with; the failure stage takes it away below
        sqlx::query("UPDATE memberships SET role = 'operator' WHERE tenant_id = $1 AND subject = 'batch-creator'")
            .bind(f.w.admin.tenant_id().as_uuid())
            .execute(&f.w.db.pool)
            .await
            .expect("promote");
        live(f);
        let created = create(f, creator, &runs[..self.items()], &format!("fault-{name}")).await;
        if matches!(self, Self::Select) {
            return created;
        }
        let selected = until_it_acts(f, exec, &created).await.expect("select");
        assert_eq!(selected, Stepped::Acted, "{}: the selection", self.name());
        match self {
            Self::Drain => {
                service(f)
                    .cancel(
                        &f.w.admin,
                        created.id,
                        "stop this batch job",
                        &RequestContext::default(),
                    )
                    .await
                    .expect("cancel the job");
            }
            Self::Fail => {
                sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'batch-creator'")
                    .bind(created.tenant_id.as_uuid())
                    .execute(&f.w.db.pool)
                    .await
                    .expect("demote");
            }
            _ => {}
        }
        created
    }

    /// What has to be true of a job that has run to its end after the stage was cut off once.
    async fn assert_ended(self, f: &Fixture, applies: &Applies, ended: &JobRecord, runs: &[Uuid]) {
        let n = self.items();
        let all = format!("{} cut off", self.name());
        let counts = &ended.counts;
        assert_eq!(counts.selected as usize, n, "{all}: selected");
        assert_eq!(
            items(f, ended.id).await.len(),
            n,
            "{all}: one item for each run, none twice"
        );
        let (state, succeeded, cancelled) = match self {
            Self::Select | Self::Item | Self::LastItem => (JobState::Completed, n, 0),
            Self::Drain => (JobState::Cancelled, 0, n),
            Self::Fail => (JobState::Failed, 0, n),
        };
        assert_eq!(ended.state, state, "{all}: how the job ended");
        assert_eq!(counts.succeeded as usize, succeeded, "{all}: succeeded");
        let item_states = items(f, ended.id).await;
        let want = if succeeded > 0 {
            "SUCCEEDED"
        } else {
            "CANCELLED"
        };
        assert!(
            item_states.iter().all(|(_, state)| state == want),
            "{all}: every item is {want}: {item_states:?}"
        );
        assert_eq!(
            counts.succeeded + counts.skipped + counts.failed + counts.cancelled,
            counts.selected,
            "{all}: the counters add up to the items"
        );
        assert_eq!(
            usize::try_from(counts.cancelled).expect("cancelled"),
            cancelled,
            "{all}: cancelled"
        );
        assert_eq!(
            endings_of(f, ended.id).await,
            1,
            "{all}: one record of the end"
        );
        // the key of an item never changes, so a repeated effect is the same effect
        let keys: BTreeSet<String> = applies
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(&format!("batch:{}:", ended.id)))
            .collect();
        let expected: BTreeSet<String> = runs[..n]
            .iter()
            .map(|run| format!("batch:{}:{run}", ended.id))
            .collect();
        match self {
            Self::Select | Self::Item | Self::LastItem => {
                assert_eq!(
                    keys, expected,
                    "{all}: the effect of each run, under one key"
                );
            }
            Self::Drain | Self::Fail => {
                assert!(keys.is_empty(), "{all}: nothing is applied: {keys:?}");
            }
        }
    }
}

async fn die_at_every_write(stage: Stage) {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;
    let creator = f.w.member("batch-creator", Role::Operator).await;
    let runs = runs_started(&f, 3).await;
    let applies = Applies::default();
    let exec = executor(&f, &applies);

    // Which writes does the step make? Watch a normal one.
    let probe = stage
        .prepare(
            &f,
            &exec,
            &creator,
            &runs,
            &format!("{}-probe", stage.name()),
        )
        .await;
    wire.watch().await;
    let acted = until_it_acts(&f, &exec, &probe).await.expect("probe");
    wire.disarm().await;
    assert_eq!(acted, Stepped::Acted, "{}: the probe", stage.name());
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "{}: a step is one transaction: {writes:?}",
        stage.name()
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    assert!(
        tables.contains("batch_jobs"),
        "{}: the step is expected to write batch_jobs: {writes:?}",
        stage.name()
    );
    if !matches!(stage, Stage::Select | Stage::Item) {
        assert!(
            tables.contains("audit_records"),
            "{}: the end of a job is audited in the same transaction: {writes:?}",
            stage.name()
        );
    }
    println!(
        "{}: the step makes {} writing statements in one transaction, on {} tables: {tables:?}",
        stage.name(),
        writes.len(),
        tables.len()
    );
    let finished = until_it_ends(&f, &exec, &probe).await;
    stage.assert_ended(&f, &applies, &finished, &runs).await;

    // Die after each of those writes, on a job of its own, and start again.
    for (_, position, table) in &writes {
        let name = format!("{}-dies-at-{position}", stage.name());
        let applies = Applies::default();
        let exec = executor(&f, &applies);
        let job = stage.prepare(&f, &exec, &creator, &runs, &name).await;
        let digests = table_digests(&f.w.db.pool).await;
        let before = self::job(&f, job.id).await;
        let items_before = items(&f, job.id).await;

        wire.arm(*position).await;
        let result = until_it_acts(&f, &exec, &job).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "{}: cut off at write {position} ({table}), the step answered {result:?}",
            stage.name()
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "{}: cut off at write {position} ({table}), a table changed",
            stage.name()
        );
        let after = self::job(&f, job.id).await;
        assert_eq!(
            (after.state, after.counts, after.next_slot_at),
            (before.state, before.counts, before.next_slot_at),
            "{}: cut off at write {position} ({table}) moved the job on",
            stage.name()
        );
        assert_eq!(items(&f, job.id).await, items_before);

        // The process restarts: a new executor, from nothing but the database.
        let applies_after = Applies::default();
        let restarted = executor(&f, &applies_after);
        let ended = until_it_ends(&f, &restarted, &job).await;
        let mut seen = applies.keys();
        seen.extend(applies_after.keys());
        stage
            .assert_ended(&f, &Applies(Arc::new(Mutex::new(seen))), &ended, &runs)
            .await;
    }

    // The matrix is the whole transaction: a step armed past its last write is not cut off.
    let beyond = stage
        .prepare(
            &f,
            &exec,
            &creator,
            &runs,
            &format!("{}-beyond", stage.name()),
        )
        .await;
    wire.arm(writes.len() + 1).await;
    let result = until_it_acts(&f, &exec, &beyond).await;
    wire.disarm().await;
    assert_eq!(
        result.expect("not cut off"),
        Stepped::Acted,
        "{}: the step makes more than the {} writes that were cut off",
        stage.name(),
        writes.len()
    );
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_page_of_the_selection_commits_nothing_and_the_page_is_taken_once()
 {
    die_at_every_write(Stage::Select).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_an_item_commits_nothing_and_the_item_ends_once() {
    die_at_every_write(Stage::Item).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_last_item_commits_nothing_and_the_job_ends_once() {
    die_at_every_write(Stage::LastItem).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_drain_commits_nothing_and_the_job_is_cancelled_once()
{
    die_at_every_write(Stage::Drain).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_failure_commits_nothing_and_the_job_fails_once() {
    die_at_every_write(Stage::Fail).await;
}

// --- duplicates and late commits ------------------------------------------------------------------

/// Waits until `n` backends wait on a lock while running a statement about batch jobs
/// (`pg_stat_activity`, not a time), then panics naming `stage` after two minutes. The world has
/// loops of its own that wait on other tables: they are not the steps this waits for.
async fn wait_for_backends_on_locks(f: &Fixture, n: i64, stage: &str) {
    within(stage, StdDuration::from_secs(120), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid() \
                   AND wait_event_type = 'Lock' AND query LIKE '%batch_jobs%'",
            )
            .fetch_one(&f.w.db.pool)
            .await
            .expect("look at the backends");
            if waiting >= n {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

#[tokio::test]
async fn two_steps_at_the_same_item_count_it_once() {
    let f = fixture().await;
    let runs = runs_started(&f, 3).await;
    let applies = Applies::default();
    let exec = Arc::new(executor(&f, &applies));
    let created = create(&f, &f.w.admin, &runs, "fault-duplicate").await;
    live(&f);
    until_it_acts(&f, &exec, &created).await.expect("select");
    f.clock.advance(Duration::seconds(1));

    // both steps read the same pending item and apply it; both then wait for the row of the job
    let mut held = f.w.db.pool.begin().await.expect("hold the job");
    sqlx::query("SELECT 1 FROM batch_jobs WHERE id = $1 FOR UPDATE")
        .bind(created.id)
        .execute(&mut *held)
        .await
        .expect("lock the job");
    let first = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    let second = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    wait_for_backends_on_locks(&f, 2, "both steps wait to record the same item").await;
    held.commit().await.expect("release the job");
    let results = [
        first.await.expect("join").expect("first"),
        second.await.expect("join").expect("second"),
    ];

    assert_eq!(
        results.iter().filter(|r| **r == Stepped::Acted).count(),
        1,
        "one of them recorded the item: {results:?}"
    );
    assert!(
        results.iter().any(|r| matches!(r, Stepped::Refused(_))),
        "the other found it ended: {results:?}"
    );
    let after = job(&f, created.id).await;
    assert_eq!(
        (after.counts.selected, after.counts.succeeded),
        (3, 1),
        "the item was counted once"
    );
    let ended: Vec<_> = items(&f, created.id)
        .await
        .into_iter()
        .filter(|(_, state)| state == "SUCCEEDED")
        .collect();
    assert_eq!(ended.len(), 1);

    // and the job goes on to the end with every run counted once
    let done = until_it_ends(&f, &exec, &created).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!((done.counts.selected, done.counts.succeeded), (3, 3));
}

#[tokio::test]
async fn a_cancel_that_commits_while_an_item_is_recorded_loses_neither_the_item_nor_the_cancel() {
    let f = fixture().await;
    let runs = runs_started(&f, 3).await;
    let applies = Applies::default();
    let exec = Arc::new(executor(&f, &applies));
    let created = create(&f, &f.w.admin, &runs, "fault-late-cancel").await;
    live(&f);
    until_it_acts(&f, &exec, &created).await.expect("select");
    f.clock.advance(Duration::seconds(1));

    // the cancel asks for the row of the job first, the step (which has applied the item) second
    let mut held = f.w.db.pool.begin().await.expect("hold the job");
    sqlx::query("SELECT 1 FROM batch_jobs WHERE id = $1 FOR UPDATE")
        .bind(created.id)
        .execute(&mut *held)
        .await
        .expect("lock the job");
    let cancel = tokio::spawn({
        let service = service(&f);
        let admin = f.w.admin.clone();
        let id = created.id;
        async move {
            service
                .cancel(
                    &admin,
                    id,
                    "stop this batch job",
                    &RequestContext::default(),
                )
                .await
        }
    });
    wait_for_backends_on_locks(&f, 1, "the cancel waits for the job").await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    wait_for_backends_on_locks(&f, 2, "the step waits to record the item").await;
    held.commit().await.expect("release the job");
    cancel.await.expect("join").expect("the cancel commits");
    let recorded = step.await.expect("join").expect("the step");

    // the item was applied before the cancel, and it is recorded as it was
    assert_eq!(
        recorded,
        Stepped::Acted,
        "the item is recorded after the cancel"
    );
    let mid = job(&f, created.id).await;
    assert_eq!(mid.state, JobState::Cancelling);
    assert_eq!(mid.counts.succeeded, 1, "the applied item is counted");

    // the rest is drained
    let done = until_it_ends(&f, &exec, &created).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert_eq!(
        (
            done.counts.selected,
            done.counts.succeeded,
            done.counts.cancelled
        ),
        (3, 1, 2)
    );
    assert_eq!(endings_of(&f, created.id).await, 1);
}

// --- a job over a thousand runs ----------------------------------------------------------------------

#[tokio::test]
async fn a_job_over_a_thousand_runs_applies_each_once_across_a_restart_and_a_cancel() {
    let f = fixture().await;
    let runs = runs_started(&f, 1_000).await;
    let applies = Applies::default();
    let first = executor(&f, &applies);
    let created = create(&f, &f.w.admin, &runs, "a-thousand-runs").await;
    live(&f);

    // the selection takes the thousand runs in two pages, the first process applies 400 of them
    let mut steps = 0;
    while job(&f, created.id).await.state == JobState::Selecting {
        f.clock.advance(Duration::seconds(1));
        first.step(&due(&created), None).await.expect("select");
        steps += 1;
        assert!(steps < 5_000, "the selection never ended");
    }
    assert_eq!(job(&f, created.id).await.counts.selected, 1_000);
    for _ in 0..400 {
        f.clock.advance(Duration::seconds(1));
        assert_eq!(
            first.step(&due(&created), None).await.expect("item"),
            Stepped::Acted
        );
    }

    // it dies; another process starts from the database alone and applies 300 more
    drop(first);
    let second = executor(&f, &applies);
    for _ in 0..300 {
        f.clock.advance(Duration::seconds(1));
        assert_eq!(
            second.step(&due(&created), None).await.expect("item"),
            Stepped::Acted
        );
    }

    // the operator cancels: what was applied stays applied, the rest is marked and nothing more runs
    service(&f)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    let ended = until_it_ends(&f, &second, &created).await;

    assert_eq!(ended.state, JobState::Cancelled);
    let c = &ended.counts;
    assert_eq!(
        (
            c.selected,
            c.succeeded,
            c.cancelled,
            c.skipped,
            c.failed,
            c.deferred
        ),
        (1_000, 700, 300, 0, 0, 0)
    );
    let by_state: Vec<(String, i64)> = sqlx::query_as(
        "SELECT state, count(*) FROM batch_items WHERE job_id = $1 GROUP BY state ORDER BY state",
    )
    .bind(created.id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("items by state");
    assert_eq!(
        by_state,
        vec![("CANCELLED".to_owned(), 300), ("SUCCEEDED".to_owned(), 700)],
        "the counters equal the outcomes of the items"
    );
    let keys: BTreeSet<String> = applies.keys().into_iter().collect();
    assert_eq!(
        applies.keys().len(),
        700,
        "each run was applied once: not repeated by the restart, not applied after the cancel"
    );
    assert_eq!(keys.len(), 700);
    assert!(
        keys.iter()
            .all(|key| key.starts_with(&format!("batch:{}:", created.id)))
    );
    assert_eq!(endings_of(&f, created.id).await, 1);
}

//! A Query is read-only. Run against the writers of a run (a worker completion, a firing timer, the
//! final completion) it never alters the journal or the scheduling rows: the run it probes ends
//! each step exactly where an identical run nobody queried ends, every answer is one coherent
//! snapshot, and the only thing a Query adds is its own access audit row.

use std::collections::BTreeMap;
use std::future::Future;

use futures_util::future::join_all;
use sagat_domain::execution::{RunId, RunStatus};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::engine::{Declared, Read, RunHold, blocked, declared, input, start};
use sagat_application::worker::ClaimedTask;

use crate::support::engine::{Session, World, activity, timer_node, with};

const QUERIES_PER_EVENT: usize = 4;

fn document(name: &str) -> Value {
    declared(
        name,
        "work",
        vec![
            with(activity("work"), "next", json!("pause")),
            with(timer_node("pause", 3_600_000), "next", json!("after")),
            activity("after"),
        ],
    )
}

/// A step as scheduling sees it: node, status, attempt, generation, what it waits on, whether it is
/// due, how often it was hinted, and whether it is ready.
type StepShape = (String, String, i32, i64, Option<String>, bool, i32, bool);

/// Everything the engine keeps about one run that a Query must not move, without ids or times that
/// differ between two runs of the same workflow.
#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    status: String,
    run_version: i64,
    journal: Vec<String>,
    steps: Vec<StepShape>,
    /// Rows per table that has a `run_id` column.
    rows: BTreeMap<String, i64>,
    outbox: Vec<(String, i64)>,
    /// Audit rows for the run by action, without the Query's own.
    audits: Vec<(String, i64)>,
}

async fn fingerprint(w: &World, run: RunId) -> Fingerprint {
    let tenant = w.admin.tenant_id().as_uuid();
    let (status, run_version): (String, i64) =
        sqlx::query_as("SELECT status, run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run");
    let journal = sqlx::query_scalar(
        "SELECT event_type FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("journal");
    let steps = sqlx::query_as(
        "SELECT node, status, attempt, generation, waiting_kind, due_at IS NOT NULL, hint_count, \
                ready_at IS NOT NULL \
         FROM steps WHERE tenant_id=$1 AND run_id=$2 ORDER BY node",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("steps");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.columns \
         WHERE table_schema='public' AND column_name='run_id' AND data_type='uuid' \
         ORDER BY table_name",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("tables");
    let mut rows = BTreeMap::new();
    for table in tables {
        // Table names come from the catalogue, not from a caller.
        let sql = format!("SELECT count(*) FROM \"{table}\" WHERE run_id = $1");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
        rows.insert(table, count);
    }
    let outbox = sqlx::query_as(
        "SELECT event_type, count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 \
         GROUP BY event_type ORDER BY event_type",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("outbox");
    let audits = sqlx::query_as(
        "SELECT action, count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action <> 'run.queried' GROUP BY action ORDER BY action",
    )
    .bind(tenant)
    .bind(run.to_string())
    .fetch_all(&w.db.pool)
    .await
    .expect("audits");
    Fingerprint {
        status,
        run_version,
        journal,
        steps,
        rows,
        outbox,
        audits,
    }
}

/// Every row of every table keyed by the run, as text, timestamps and all: a Query that touched
/// even `updated_at` shows here.
async fn row_images(w: &World, run: RunId) -> BTreeMap<String, Vec<String>> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.columns \
         WHERE table_schema='public' AND column_name='run_id' AND data_type='uuid' \
         ORDER BY table_name",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("tables");
    let mut images = BTreeMap::new();
    for table in tables {
        let sql = format!("SELECT x::text FROM \"{table}\" x WHERE run_id = $1 ORDER BY 1");
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(run.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("rows");
        images.insert(table, rows);
    }
    let run_row: Vec<String> = sqlx::query_scalar("SELECT r::text FROM runs r WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("run row");
    images.insert("runs".to_owned(), run_row);
    images
}

/// A Query and what it was answered.
#[derive(Debug)]
struct Asked {
    operation: &'static str,
    argument: i64,
    read: Read,
}

fn queries(fixture: &Declared, count: usize) -> impl Future<Output = Vec<Asked>> + use<'_> {
    let run = fixture.run;
    async move {
        join_all((0..count).map(|index| {
            let operation = if index % 2 == 0 {
                "balance.read"
            } else {
                "work.read"
            };
            let argument = i64::try_from(index).expect("small");
            let ops = fixture.ops.clone();
            async move {
                let read = ops
                    .query(run, operation, &json!(argument))
                    .await
                    .expect("the query is served");
                Asked {
                    operation,
                    argument,
                    read,
                }
            }
        }))
        .await
    }
}

/// Runs `event` and a burst of Queries so that all of them wait on the run row together, then lets
/// the lock go: the event and the Queries race for it.
async fn event_with_queries<T>(
    fixture: &Declared,
    observer: &PgPool,
    event: impl Future<Output = T>,
) -> (T, Vec<Asked>) {
    let mut event = Box::pin(event);
    let mut burst = Box::pin(queries(fixture, QUERIES_PER_EVENT));
    let hold = RunHold::take(observer, &fixture.w, fixture.run).await;
    let waiters = i64::try_from(QUERIES_PER_EVENT).expect("small") + 1;
    tokio::select! {
        _ = &mut event => panic!("the event finished while the run row was held"),
        _ = &mut burst => panic!("the queries finished while the run row was held"),
        () = blocked(observer, waiters) => {}
    }
    hold.release().await;
    tokio::join!(event, burst)
}

/// The three writers of the run, in order: a completion, a firing timer, the final completion.
#[derive(Clone, Copy, Debug)]
enum Event {
    Work,
    Timer,
    After,
}

/// What an event needs in hand before it starts. A worker polls for its task first: the poll skips
/// a run whose row is locked, so it does not belong inside the race.
async fn prepare(fixture: &Declared, session: &Session, event: Event) -> Option<ClaimedTask> {
    match event {
        Event::Work | Event::After => Some(fixture.w.one_task(session).await),
        Event::Timer => {
            fixture.w.age_due(fixture.run, "pause").await;
            None
        }
    }
}

async fn happen(fixture: &Declared, session: &Session, event: Event, task: Option<ClaimedTask>) {
    match (event, task) {
        (Event::Work, Some(task)) => {
            fixture
                .w
                .complete(session, &task, &json!({"amount": 7}))
                .await
                .expect("work completion");
        }
        (Event::After, Some(task)) => {
            fixture
                .w
                .complete(session, &task, &json!({"done": true}))
                .await
                .expect("final completion");
        }
        (Event::Timer, None) => {
            let report = fixture.w.scheduler.tick().await;
            assert_eq!(report.timers_fired, 1, "{report:?}");
        }
        (event, _) => panic!("{event:?} was not prepared"),
    }
}

async fn observer(fixture: &Declared) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&fixture.w.db.url)
        .await
        .expect("observer pool")
}

#[tokio::test]
async fn queries_racing_the_writers_of_a_run_change_nothing_but_their_own_audit() {
    let control = start(document("operations-query-control"), &input(10)).await;
    let probed = start(document("operations-query-probed"), &input(10)).await;
    let probed_observer = observer(&probed).await;
    let activities = ["act.work", "act.after"];
    let control_session = control.w.session(&activities).await;
    let probed_session = probed.w.session(&activities).await;

    assert_eq!(
        fingerprint(&control.w, control.run).await,
        fingerprint(&probed.w, probed.run).await,
        "the two runs begin alike"
    );

    let mut reads: Vec<Asked> = Vec::new();
    let mut committed_versions = vec![fingerprint(&probed.w, probed.run).await.run_version];
    let mut completed_at = None;
    for event in [Event::Work, Event::Timer, Event::After] {
        let task = prepare(&control, &control_session, event).await;
        happen(&control, &control_session, event, task).await;
        let task = prepare(&probed, &probed_session, event).await;
        // The poll that leased the task committed too: a Query may answer from just before the event.
        committed_versions.push(fingerprint(&probed.w, probed.run).await.run_version);
        let ((), burst) = event_with_queries(
            &probed,
            &probed_observer,
            happen(&probed, &probed_session, event, task),
        )
        .await;
        reads.extend(burst);

        let expected = fingerprint(&control.w, control.run).await;
        let observed = fingerprint(&probed.w, probed.run).await;
        assert_eq!(
            observed, expected,
            "{event:?}: the queried run diverged from the run nobody queried"
        );
        committed_versions.push(observed.run_version);
        if matches!(event, Event::Work) {
            completed_at = Some(observed.run_version);
        }

        // With the writers quiet, a further burst leaves every row byte for byte as it was,
        // timestamps included.
        let before = row_images(&probed.w, probed.run).await;
        let burst = queries(&probed, QUERIES_PER_EVENT).await;
        assert_eq!(
            row_images(&probed.w, probed.run).await,
            before,
            "{event:?}: a quiet query burst changed a row"
        );
        reads.extend(burst);
    }
    assert_eq!(
        probed.w.run_status(probed.run).await,
        RunStatus::Succeeded,
        "the probed run finished"
    );
    probed.w.assert_consistent(probed.run).await;
    control.w.assert_consistent(control.run).await;

    // Every answer is one coherent snapshot of a committed version: the completed result shows
    // exactly from the version that committed it on, and the state never moved.
    let completed_at = completed_at.expect("the work completion committed");
    for asked in &reads {
        let read = &asked.read;
        let version = i64::try_from(read.run_version).expect("version");
        assert!(
            committed_versions.contains(&version),
            "answered a version that was never committed: {asked:?} not in {committed_versions:?}"
        );
        assert_eq!(read.state_revision, 0, "{asked:?}");
        let seen = read.result.as_i64().expect("integer result");
        let expected = match asked.operation {
            "balance.read" => 10 + asked.argument,
            _ if version >= completed_at => 7 + asked.argument,
            _ => asked.argument,
        };
        assert_eq!(seen, expected, "an incoherent snapshot: {asked:?}");
    }

    // The access audit is the only addition. An identical repeated access is written once (the
    // audit collapses repeats a caller can make at will), so the rows are one per operation asked,
    // however many times it was asked.
    let queried: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='run.queried' ORDER BY reason",
    )
    .bind(probed.w.admin.tenant_id().as_uuid())
    .bind(probed.run.to_string())
    .fetch_all(&probed.w.db.pool)
    .await
    .expect("access audit");
    assert_eq!(queried, ["operation=balance.read", "operation=work.read"]);
    assert!(reads.len() > queried.len(), "the audit collapsed repeats");
    let untouched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='run.queried'",
    )
    .bind(control.w.admin.tenant_id().as_uuid())
    .bind(control.run.to_string())
    .fetch_one(&control.w.db.pool)
    .await
    .expect("control audit");
    assert_eq!(untouched, 0, "nobody queried the control run");
}

/// The detectors above must be able to fail: a write as small as touching one timestamp moves the
/// row images, and one more journal event moves the fingerprint.
#[tokio::test]
async fn the_detectors_see_a_touched_timestamp_and_an_extra_event() {
    let fixture = start(document("operations-query-detector"), &input(10)).await;
    let w = &fixture.w;
    let images = row_images(w, fixture.run).await;
    let fingerprint_before = fingerprint(w, fixture.run).await;

    sqlx::query(
        "UPDATE steps SET updated_at = updated_at + interval '1 microsecond' WHERE run_id = $1",
    )
    .bind(fixture.run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("touch a step");
    assert_ne!(row_images(w, fixture.run).await, images);

    sqlx::query(
        "INSERT INTO run_events (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         SELECT tenant_id, run_id, max(seq) + 1, 'step.touched', '{}'::jsonb, 'test', now() \
         FROM run_events WHERE run_id = $1 GROUP BY tenant_id, run_id",
    )
    .bind(fixture.run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("add an event");
    assert_ne!(fingerprint(w, fixture.run).await, fingerprint_before);
}

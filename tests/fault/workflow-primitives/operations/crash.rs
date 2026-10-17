//! An Update that dies partway: its future dropped at every await point, and its database
//! connection killed while it waits for the run lock and again inside the serialized section. The
//! change is all there or not there, no transaction is left open, and a restarted store (a new
//! pool and a new store over the same database) applies the same key exactly once.
//!
//! A process cannot be killed from inside a test, so these are the two ways the database sees one
//! die: the client stops driving a statement (a dropped future), or its connection ends under it
//! (`pg_terminate_backend`). Neither claims to cover a crash of the PostgreSQL server itself.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use sagat_domain::execution::{RunId, UpdateReceiptStatus};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::engine::{
    Declared, Ops, RunHold, blocked, declared, input, journal, replace, start, update_evidence,
};
use crate::support::engine::{activity, with};

/// Polls its future at most `polls` times and gives it up, dropped mid-flight, if it is still
/// pending after the last poll. Every poll that returns pending is one await point of the future,
/// so counting up from one cuts it at each of them in turn.
struct CutAfter<F> {
    inner: Pin<Box<F>>,
    polls: usize,
}

impl<F: Future> Future for CutAfter<F> {
    type Output = Option<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.polls == 0 {
            return Poll::Ready(None);
        }
        self.polls -= 1;
        match self.inner.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Some(output)),
            Poll::Pending if self.polls == 0 => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

fn document(name: &str) -> serde_json::Value {
    declared(
        name,
        "work",
        vec![
            with(activity("work"), "next", json!("hold")),
            activity("hold"),
        ],
    )
}

async fn one_connection(fixture: &Declared) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&fixture.w.db.url)
        .await
        .expect("one-connection pool")
}

/// Nothing is left open on the connection of `pool`: on its own a statement is its own
/// transaction, so `now()` moves; inside a forgotten transaction it would not. The database also
/// reports no session idle inside a transaction.
async fn assert_no_open_transaction(fixture: &Declared, pool: &PgPool, what: &str) {
    let first: time::OffsetDateTime = sqlx::query_scalar("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("first statement");
    let second: time::OffsetDateTime = sqlx::query_scalar("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("second statement");
    assert_ne!(first, second, "{what} left its connection in a transaction");
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND state LIKE 'idle in transaction%'",
    )
    .fetch_one(&fixture.w.db.pool)
    .await
    .expect("sessions");
    assert_eq!(open, 0, "{what} left a session idle in a transaction");
}

/// All of the Update or none of it: the projection, the receipt, the journal event, the outbox row
/// and the accepted-update audit row stand or fall together. Returns the revision they stand at.
async fn assert_whole_or_nothing(fixture: &Declared, run: RunId, what: &str) -> i64 {
    let (revision, receipts, events, outbox) = update_evidence(&fixture.w, run).await;
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='run.updated'",
    )
    .bind(fixture.w.admin.tenant_id().as_uuid())
    .bind(run.to_string())
    .fetch_one(&fixture.w.db.pool)
    .await
    .expect("audit rows");
    assert!(
        revision == 0 || revision == 1,
        "{what}: revision {revision}"
    );
    assert_eq!(
        (receipts, events, outbox, audits),
        (revision, revision, revision, revision),
        "{what}: a partly committed update"
    );
    revision
}

/// The restarted process: a new pool and a new store over the same database replay the key. The
/// receipt is applied exactly once, and a replay says so.
async fn restart_and_replay(
    fixture: &Declared,
    run: RunId,
    key: &str,
    committed_before: bool,
    what: &str,
) {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&fixture.w.db.url)
        .await
        .expect("the restarted process connects");
    let restarted = Ops::on(&fixture.w, pool);
    let answer = restarted
        .update(run, "balance.replace", key, &replace(41), Some(0))
        .await
        .unwrap_or_else(|error| panic!("{what}: the restarted store failed: {error:?}"));
    assert_eq!(answer.status(), UpdateReceiptStatus::Applied, "{what}");
    assert_eq!(answer.result, Some(json!(41)), "{what}");
    assert_eq!(answer.resolution.replayed(), committed_before, "{what}");
    assert_eq!(
        update_evidence(&fixture.w, run).await,
        (1, 1, 1, 1),
        "{what}"
    );
    let again = restarted
        .update(run, "balance.replace", key, &replace(41), Some(0))
        .await
        .expect("a second replay");
    assert!(again.resolution.replayed(), "{what}");
    assert_eq!(
        update_evidence(&fixture.w, run).await,
        (1, 1, 1, 1),
        "{what}"
    );
    fixture.w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_update_cut_off_at_every_await_point_commits_whole_or_not_at_all() {
    let fixture = start(document("operations-cut-off"), &input(10)).await;
    let workflow = fixture.workflow;
    let pool = one_connection(&fixture).await;
    let cut_store = Ops::on(&fixture.w, pool.clone());
    let mut cut_rounds = 0;
    let mut committed_when_cut = 0;
    let mut finished = false;

    for polls in 1..=400 {
        // A new run for every cut point, so each starts from the same untouched state.
        let run = fixture.w.start(workflow, &input(10)).await;
        let command =
            cut_store.update_command(run, "balance.replace", "cut-off", &replace(41), Some(0));
        let outcome = CutAfter {
            inner: Box::pin(cut_store.apply(&command)),
            polls,
        }
        .await;
        let what = format!("cut after {polls} polls");

        assert_no_open_transaction(&fixture, &pool, &what).await;
        let revision = assert_whole_or_nothing(&fixture, run, &what).await;
        match outcome {
            None => {
                cut_rounds += 1;
                committed_when_cut += revision;
            }
            Some(answer) => {
                let answer = answer.expect("an uncut update succeeds");
                assert_eq!(answer.status(), UpdateReceiptStatus::Applied, "{what}");
                assert_eq!(revision, 1, "{what}");
                finished = true;
            }
        }
        restart_and_replay(&fixture, run, "cut-off", revision == 1, &what).await;
        if finished {
            break;
        }
    }

    assert!(finished, "the update never completed within 400 polls");
    assert!(
        cut_rounds >= 8,
        "only {cut_rounds} cut points: the test proves little"
    );
    // The late cuts landed after the commit (the response was lost), the early ones before it:
    // both sides of the commit point were exercised.
    assert!(committed_when_cut >= 1, "no cut landed after the commit");
    assert!(
        committed_when_cut < cut_rounds,
        "no cut landed before the commit"
    );
}

/// A connection killed while the update waits for the run lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_killed_while_queued_for_the_run_lock_leaves_nothing_and_replays_once() {
    let fixture = start(document("operations-kill-queued"), &input(10)).await;
    let run = fixture.run;
    let pool = one_connection(&fixture).await;
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .expect("backend pid");
    let ops = Ops::on(&fixture.w, pool.clone());
    let hold = RunHold::take(&fixture.w.db.pool, &fixture.w, run).await;
    let command = ops.update_command(run, "balance.replace", "kill-queued", &replace(41), Some(0));
    let update = tokio::spawn({
        let ops = ops.clone();
        async move { ops.apply(&command).await }
    });
    blocked(&fixture.w.db.pool, 1).await;
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(backend)
        .fetch_one(&fixture.w.db.pool)
        .await
        .expect("terminate the update's connection");
    assert!(killed);
    let outcome = update.await.expect("update task");
    assert!(outcome.is_err(), "a killed connection fails the update");
    hold.release().await;

    assert_eq!(update_evidence(&fixture.w, run).await, (0, 0, 0, 0));
    assert_no_open_transaction(&fixture, &fixture.w.db.pool, "the killed update").await;
    restart_and_replay(&fixture, run, "kill-queued", false, "killed while queued").await;
}

/// A connection killed inside the serialized section: the run is locked, the snapshot is read and
/// the decision is made, and nothing is written yet.
///
/// The update runs on a thread of its own, with its own runtime and its own one-connection pool,
/// because its gate blocks the thread that runs it; the rest of the test stays on the test runtime.
#[tokio::test]
async fn a_connection_killed_after_the_decision_leaves_nothing_and_replays_once() {
    let fixture = start(document("operations-kill-decided"), &input(10)).await;
    let run = fixture.run;
    let command = fixture.ops.update_command(
        run,
        "balance.replace",
        "kill-decided",
        &replace(41),
        Some(0),
    );
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<i32>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let ops = fixture.ops.clone();
    let url = fixture.w.db.url.clone();
    let update = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the update's runtime");
        runtime.block_on(async move {
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .expect("one-connection pool");
            let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&pool)
                .await
                .expect("backend pid");
            ops.over(pool)
                .apply_gated(&command, move || {
                    entered_tx.send(backend).expect("announce the decision");
                    release_rx.recv().expect("wait to be released");
                })
                .await
        })
    });
    let backend = entered_rx.await.expect("the update reached its decision");
    let inside: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND state = 'idle in transaction'",
    )
    .bind(backend)
    .fetch_one(&fixture.w.db.pool)
    .await
    .expect("the update's session");
    assert_eq!(inside, 1, "the update is inside its transaction");
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(backend)
        .fetch_one(&fixture.w.db.pool)
        .await
        .expect("terminate the update's connection");
    assert!(killed);
    release_tx.send(()).expect("release the decision");
    let outcome = update.join().expect("update thread");
    assert!(outcome.is_err(), "a killed connection fails the update");

    assert_eq!(update_evidence(&fixture.w, run).await, (0, 0, 0, 0));
    assert_no_open_transaction(&fixture, &fixture.w.db.pool, "the killed update").await;
    let events = journal(&fixture.w, run).await;
    assert!(
        events.iter().all(|(_, event)| event != "state.updated"),
        "{events:?}"
    );
    restart_and_replay(
        &fixture,
        run,
        "kill-decided",
        false,
        "killed after decision",
    )
    .await;
}

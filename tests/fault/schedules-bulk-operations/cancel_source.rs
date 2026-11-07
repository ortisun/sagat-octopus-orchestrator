//! `CANCEL_SOURCE` under crashes and races (`T018-057`, `SC-018-012`, `FR-018-038`, AGENTS rule 9): the request is one
//! transaction (the entry, the receipt, the audit record and the cancellation of the source), so a process killed
//! between two of its statements leaves nothing behind and the same request sent again cancels the source once and
//! makes one entry; the completion is one transaction (the run, its prefix, the entry's new state, the audit record),
//! so a process killed in it leaves the entry pending and the step sent again makes one run. The connection is
//! killed inside PostgreSQL after each writing statement in turn (`tripwire.rs`). The races are made with held row
//! locks and the queue of the lock, never with sleeps. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::reset_completion::{
    Completed, Completion, DueReset, ResetCompleter, ResetCompletions,
};
use sagat_application::runs::{
    PendingReset, RerunBusinessId, ResetMade, ResetRequest, ResetSourcePolicy, ResetState,
};
use sagat_application::schedules::ShardFence;
use sagat_application::worker::ClaimedTask;
use sagat_domain::execution::{ResetId, ResetReapply, RunId, RunStatus};
use sagat_domain::identity::Principal;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore};
use serde_json::json;
use uuid::Uuid;

use crate::support::bounded::within;
use crate::support::engine::{Session, World, activity, ctx, document, with};
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

const WHY: &str = "the second step used a wrong price list";

struct Completions(Arc<World>);

impl ResetCompletions for Completions {
    async fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        self.0
            .runs
            .complete_reset(principal, entry, fence, ctx)
            .await
    }
}

type Completer = ResetCompleter<PgRunStore, Completions, PgIdentityStore, SystemClock>;

fn completer(f: &Fixture) -> Completer {
    ResetCompleter::new(
        f.w.store.clone(),
        Completions(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        SystemClock,
    )
}

/// A source of two activities that has not ended: `a` is done and `b`, the last, is leased to a worker. When the
/// worker completes `b` the source ends on its own.
struct Open {
    run: RunId,
    b: ClaimedTask,
    after_a: u64,
}

/// The worker that serves the activities of every source of a test: sessions of one identity are limited.
async fn worker(f: &Fixture) -> Session {
    f.w.session(&["act.a", "act.b"]).await
}

async fn open_pair(f: &Fixture, session: &Session) -> Open {
    let workflow =
        f.w.publish(document(
            &format!("cancel-source-fault-{}", Uuid::now_v7()),
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await
        .0;
    let run = f.w.start(workflow, &json!({"k": 1})).await;
    let a = f.w.one_task_of(session, "act.a").await;
    f.w.complete(session, &a, &json!({"done": "a"}))
        .await
        .expect("a completes");
    let b = f.w.one_task_of(session, "act.b").await;
    let after_a: i64 = sqlx::query_scalar(
        "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' ORDER BY seq LIMIT 1",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the completion of a");
    Open {
        run,
        b,
        after_a: u64::try_from(after_a).expect("small"),
    }
}

async fn finish(f: &Fixture, session: &Session, open: &Open) {
    f.w.complete(session, &open.b, &json!({"done": "b"}))
        .await
        .expect("b completes");
    assert!(f.w.run_status(open.run).await.is_terminal());
}

fn asking(sequence: u64) -> ResetRequest<'static> {
    ResetRequest {
        sequence,
        reason: WHY,
        acknowledge_reexecution: true,
        business: RerunBusinessId::Keep,
        reapply: ResetReapply::None,
        source_policy: ResetSourcePolicy::CancelSource,
    }
}

async fn request(f: &Fixture, run: RunId, sequence: u64, key: &str) -> Result<ResetMade, AppError> {
    f.w.runs
        .reset(&f.w.admin, run, asking(sequence), key, &ctx())
        .await
}

fn due(f: &Fixture, id: ResetId) -> DueReset {
    DueReset {
        tenant_id: f.w.admin.tenant_id(),
        id,
    }
}

async fn made_from(f: &Fixture, source: RunId) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE rerun_of = $1 ORDER BY id")
        .bind(source.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the runs")
}

async fn entries(f: &Fixture, source: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE source_run_id = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the register")
}

async fn stops(f: &Fixture, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the stop requests")
}

async fn receipts(f: &Fixture, key: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM command_dedupe WHERE idempotency_key = $1")
        .bind(key)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the receipts")
}

/// Waits until `n` backends of this database wait on a lock (`pg_stat_activity`, not a time), at most two minutes.
async fn wait_for_backends_on_locks(f: &Fixture, n: i64, stage: &str) {
    within(stage, std::time::Duration::from_secs(120), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid() \
                   AND wait_event_type = 'Lock'",
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

/// Room for `n` resets that wait in the namespace at once: the deployment ceiling first, then the limit.
async fn room_for_pending(f: &Fixture, n: i64) {
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'pending_resets', $2, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'pending_resets', $3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the pending resets");
}

// ------------------------------------------------------------------------------------------- the request

#[tokio::test]
async fn a_request_that_dies_at_any_write_leaves_nothing_and_sent_again_cancels_once_and_makes_one_entry()
 {
    let f = fixture().await;
    let session = worker(&f).await;
    room_for_pending(&f, 64).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // a first request of a tenant makes writes that later ones do not; one is made first so that the probe shows the
    // writes of an ordinary request
    let warm = open_pair(&f, &session).await;
    request(&f, warm.run, warm.after_a, "cancel-source-warm-up-key")
        .await
        .expect("the warm-up request");

    let probe = open_pair(&f, &session).await;
    wire.watch().await;
    request(&f, probe.run, probe.after_a, "cancel-source-probe-key")
        .await
        .expect("the probe request");
    wire.disarm().await;
    let writes = wire.writes().await;
    assert_eq!(
        writes
            .iter()
            .map(|(xid, _, _)| *xid)
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "a request is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in [
        "run_resets",
        "audit_records",
        "command_dedupe",
        "run_events",
        "runs",
    ] {
        assert!(
            tables.contains(table),
            "a request is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "a request makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let open = open_pair(&f, &session).await;
        let key = format!("cancel-source-dies-at-{position}-key");
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = request(&f, open.run, open.after_a, &key).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the request answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(stops(&f, open.run).await, 0, "write {position}");
        assert_eq!(entries(&f, open.run).await, 0, "write {position}");
        assert_eq!(receipts(&f, &key).await, 0, "write {position}");
        assert!(!f.w.run_status(open.run).await.is_terminal());

        // sent again: the source is cancelled once, there is one entry, one receipt
        let again = request(&f, open.run, open.after_a, &key)
            .await
            .unwrap_or_else(|error| panic!("write {position} ({table}), sent again: {error:?}"));
        assert_eq!(again.reset.state, ResetState::PendingSource);
        assert_eq!(stops(&f, open.run).await, 1, "write {position}");
        assert_eq!(entries(&f, open.run).await, 1, "write {position}");
        assert_eq!(receipts(&f, &key).await, 1, "write {position}");

        // and a third time is the answer of the first
        let third = request(&f, open.run, open.after_a, &key)
            .await
            .expect("a replay of the request");
        assert_eq!(third.reset.id, again.reset.id, "write {position}");
        assert_eq!(stops(&f, open.run).await, 1, "write {position}: still one");
        assert_eq!(
            entries(&f, open.run).await,
            1,
            "write {position}: still one"
        );
    }
}

// ----------------------------------------------------------------------------------------- the completion

#[tokio::test]
async fn a_completion_that_dies_at_any_write_leaves_the_entry_pending_and_sent_again_makes_one_run()
{
    let f = fixture().await;
    let session = worker(&f).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // a pending entry whose source has ended
    async fn ready(f: &Fixture, session: &Session, key: &str) -> (RunId, ResetId) {
        let open = open_pair(f, session).await;
        let made = request(f, open.run, open.after_a, key)
            .await
            .expect("the request");
        finish(f, session, &open).await;
        (open.run, made.reset.id)
    }

    let (warm_source, warm) = ready(&f, &session, "completion-warm-up-key").await;
    assert_eq!(
        completer(&f)
            .step(&due(&f, warm), None)
            .await
            .expect("warm-up"),
        Completed::Created
    );
    assert_eq!(made_from(&f, warm_source).await.len(), 1);

    let (probe_source, probe) = ready(&f, &session, "completion-probe-key").await;
    wire.watch().await;
    assert_eq!(
        completer(&f)
            .step(&due(&f, probe), None)
            .await
            .expect("the probe"),
        Completed::Created
    );
    wire.disarm().await;
    let writes = wire.writes().await;
    assert_eq!(
        writes
            .iter()
            .map(|(xid, _, _)| *xid)
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "a completion is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in [
        "runs",
        "run_events",
        "steps",
        "run_resets",
        "audit_records",
        "command_dedupe",
        "event_outbox",
    ] {
        assert!(
            tables.contains(table),
            "a completion is expected to write {table}: {writes:?}"
        );
    }
    assert_eq!(made_from(&f, probe_source).await.len(), 1);
    println!(
        "a completion makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let (source, id) = ready(&f, &session, &format!("completion-dies-at-{position}-key")).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = completer(&f).step(&due(&f, id), None).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the step answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert!(made_from(&f, source).await.is_empty(), "write {position}");

        // sent again: one run, the entry made
        assert_eq!(
            completer(&f)
                .step(&due(&f, id), None)
                .await
                .unwrap_or_else(|error| panic!(
                    "write {position} ({table}), sent again: {error:?}"
                )),
            Completed::Created,
            "write {position}"
        );
        let runs = made_from(&f, source).await;
        assert_eq!(runs.len(), 1, "write {position}");
        let state: (String, Option<Uuid>) =
            sqlx::query_as("SELECT state, new_run_id FROM run_resets WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the entry");
        assert_eq!(
            state,
            ("CREATED".to_owned(), Some(runs[0])),
            "write {position}"
        );
        f.w.store
            .verify_run(f.w.admin.tenant_id(), RunId::new(runs[0]))
            .await
            .expect("the projection equals a replay of the journal");

        // and a third time finds nothing to do
        assert_eq!(
            completer(&f)
                .step(&due(&f, id), None)
                .await
                .expect("a third step"),
            Completed::Idle,
            "write {position}"
        );
        assert_eq!(made_from(&f, source).await.len(), 1, "write {position}");
    }
}

// ------------------------------------------------------------------------------------------------- races

#[tokio::test]
async fn the_source_ending_on_its_own_before_the_request_gets_its_row_makes_an_ordinary_reset() {
    let f = fixture().await;
    let session = worker(&f).await;
    let open = open_pair(&f, &session).await;
    // The first waiter for the row is the worker that completes the last step, the second is the request. The
    // request has read the source open and then waits; when it gets the row the source has ended.
    //
    // Both futures are driven at once, and the row is released only when two backends wait for it.
    let mut held = f.w.db.pool.begin().await.expect("hold the run");
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(open.run.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the row of the source");
    let output = json!({"done": "b"});
    let worker = f.w.complete(&session, &open.b, &output);
    let asked = async {
        wait_for_backends_on_locks(&f, 1, "the worker waits for the row of the source").await;
        request(
            &f,
            open.run,
            open.after_a,
            "cancel-source-race-ended-first-key",
        )
        .await
    };
    let release = async {
        wait_for_backends_on_locks(&f, 2, "the request waits behind the worker").await;
        held.commit().await.expect("release the source");
    };
    let (completed, made, ()) = tokio::join!(worker, asked, release);

    completed.expect("the worker completed the step");
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Succeeded);
    let made = made.expect("the request");
    // the source had ended when the request got its row: a reset at once, nothing was cancelled, nothing waits
    assert_eq!(made.reset.state, ResetState::Created);
    let run = made.run.expect("the run is made at once");
    assert_eq!(made.reset.new_run, Some(run.info.run_id));
    assert_eq!(stops(&f, open.run).await, 0, "the source was not cancelled");
    assert_eq!(entries(&f, open.run).await, 1);
    assert_eq!(made_from(&f, open.run).await.len(), 1);
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run.info.run_id)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn the_request_committing_before_the_source_ends_leaves_an_entry_the_pass_completes() {
    let f = fixture().await;
    let session = worker(&f).await;
    let open = open_pair(&f, &session).await;
    // The first waiter for the row is the request, the second the worker: the source is cancelled with its last step
    // in flight, and the step that completes after the cancellation ends it as cancelled.
    let mut held = f.w.db.pool.begin().await.expect("hold the run");
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(open.run.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the row of the source");
    let asked = request(
        &f,
        open.run,
        open.after_a,
        "cancel-source-race-request-first-key",
    );
    let worker = async {
        wait_for_backends_on_locks(&f, 1, "the request waits for the row of the source").await;
        f.w.complete(&session, &open.b, &json!({"done": "b"})).await
    };
    let release = async {
        wait_for_backends_on_locks(&f, 2, "the worker waits behind the request").await;
        held.commit().await.expect("release the source");
    };
    let (made, completed, ()) = tokio::join!(asked, worker, release);

    let made = made.expect("the request");
    completed.expect("the worker completed the step");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert_eq!(stops(&f, open.run).await, 1);
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelled);
    assert!(
        made_from(&f, open.run).await.is_empty(),
        "the pass has not run yet"
    );
    assert_eq!(
        completer(&f)
            .step(&due(&f, made.reset.id), None)
            .await
            .expect("the step"),
        Completed::Created
    );
    assert_eq!(made_from(&f, open.run).await.len(), 1);
    assert_eq!(
        f.w.run_status(open.run).await,
        RunStatus::Cancelled,
        "the source never reopens"
    );
}

#[tokio::test]
async fn two_engines_completing_the_same_entry_make_one_run_and_the_second_is_refused() {
    let f = fixture().await;
    let session = worker(&f).await;
    let open = open_pair(&f, &session).await;
    let made = request(&f, open.run, open.after_a, "cancel-source-two-engines-key")
        .await
        .expect("the request");
    finish(&f, &session, &open).await;

    // both engines read the entry as pending and go for its row; one holds it first
    let mut held = f.w.db.pool.begin().await.expect("hold the entry");
    sqlx::query("SELECT id FROM run_resets WHERE id = $1 FOR UPDATE")
        .bind(made.reset.id.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the entry");
    let (one, two) = (completer(&f), completer(&f));
    let (a, b, ()) = tokio::join!(
        async { one.step(&due(&f, made.reset.id), None).await },
        async { two.step(&due(&f, made.reset.id), None).await },
        async {
            wait_for_backends_on_locks(&f, 2, "both engines wait for the entry").await;
            // what they wait on is the lock of the entry itself, taken before anything else is written: not a
            // later statement that happens to meet the same row
            let waiting: Vec<String> = sqlx::query_scalar(
                "SELECT query FROM pg_stat_activity WHERE datname = current_database() \
                   AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
            )
            .fetch_all(&f.w.db.pool)
            .await
            .expect("look at the waiting backends");
            assert_eq!(waiting.len(), 2, "{waiting:?}");
            for query in &waiting {
                assert!(
                    query.contains("FROM run_resets") && query.contains("FOR UPDATE"),
                    "an engine waits for the entry with a lock on it: {query}"
                );
            }
            held.commit().await.expect("release the entry");
        }
    );

    let outcomes = [a.expect("one step"), b.expect("the other step")];
    let created = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Created))
        .count();
    let refused = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Refused(reason) if reason.contains("completed")))
        .count();
    assert_eq!((created, refused), (1, 1), "{outcomes:?}");
    assert_eq!(made_from(&f, open.run).await.len(), 1, "one run");
    let completed_entries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_resets WHERE source_run_id = $1 AND state = 'CREATED'",
    )
    .bind(open.run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("entries");
    assert_eq!(completed_entries, 1);
}

#[tokio::test]
async fn a_requester_revoked_while_the_run_is_admitted_stops_it_under_the_lock_of_their_membership()
{
    revoked_while_admitted("viewer", "opal").await;
}

/// The member who asked is demoted to `role` (uncommitted) while the completion goes to admit the run: the admission
/// waits for the membership it must lock, the demotion returns, and the run is not made.
async fn revoked_while_admitted(role: &str, name: &str) {
    let f = fixture().await;
    let session = worker(&f).await;
    let operator =
        f.w.member(name, sagat_domain::identity::Role::Operator)
            .await;
    let open = open_pair(&f, &session).await;
    let made =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a),
                &format!("cancel-source-revoked-to-{role}-key"),
                &ctx(),
            )
            .await
            .expect("an operator may reset");
    finish(&f, &session, &open).await;

    // The completion has read the membership (an operator) and goes on to admit the run; the revocation is
    // uncommitted until the admission waits for the membership it must lock. The revocation that returns first wins.
    let mut revoking = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query("UPDATE memberships SET role = $3 WHERE tenant_id = $1 AND subject = $2")
        .bind(operator.tenant_id().as_uuid())
        .bind(operator.subject().as_str())
        .bind(role)
        .execute(&mut *revoking)
        .await
        .expect("revoke, uncommitted");
    let (step, ()) = tokio::join!(
        async { completer(&f).step(&due(&f, made.reset.id), None).await },
        async {
            wait_for_backends_on_locks(&f, 1, "the admission waits for the membership").await;
            revoking.commit().await.expect("the revocation returns");
        }
    );
    assert_eq!(
        step.expect("the step"),
        Completed::Failed("AUTHORITY_REVOKED"),
        "a revocation that has returned stops the run"
    );
    assert!(made_from(&f, open.run).await.is_empty());
}

#[tokio::test]
async fn a_requester_demoted_to_a_role_that_starts_runs_but_does_not_operate_them_is_revoked_too() {
    revoked_while_admitted("developer", "oren").await;
}

#[tokio::test]
async fn two_equal_requests_meeting_at_the_row_make_one_entry_and_both_are_answered_with_it() {
    let f = fixture().await;
    let session = worker(&f).await;
    let open = open_pair(&f, &session).await;
    // both read the source open and found no receipt, and both go for its row; one holds it first
    let mut held = f.w.db.pool.begin().await.expect("hold the run");
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(open.run.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the row of the source");
    let key = "cancel-source-two-requests-key";
    let (one, other, ()) = tokio::join!(
        request(&f, open.run, open.after_a, key),
        request(&f, open.run, open.after_a, key),
        async {
            wait_for_backends_on_locks(&f, 2, "both requests wait for the row of the source").await;
            held.commit().await.expect("release the source");
        }
    );
    let (one, other) = (one.expect("one request"), other.expect("the other request"));
    assert_eq!(
        one.reset.id, other.reset.id,
        "both are answered with the one entry"
    );
    assert_eq!(one.reset.state, ResetState::PendingSource);
    assert_eq!(other.reset.state, ResetState::PendingSource);
    assert_eq!(stops(&f, open.run).await, 1, "cancelled once");
    assert_eq!(entries(&f, open.run).await, 1, "one entry");
    assert_eq!(receipts(&f, key).await, 1, "one receipt");
}

#[tokio::test]
async fn two_engines_failing_the_same_entry_record_one_failure_and_the_second_is_refused() {
    let f = fixture().await;
    let session = worker(&f).await;
    let operator =
        f.w.member("fay", sagat_domain::identity::Role::Operator)
            .await;
    let open = open_pair(&f, &session).await;
    let made =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a),
                "cancel-source-two-failures-key",
                &ctx(),
            )
            .await
            .expect("an operator may reset");
    finish(&f, &session, &open).await;
    // the member who asked is gone: both engines mean to fail the entry
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = $2")
        .bind(operator.tenant_id().as_uuid())
        .bind(operator.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("remove the member");

    let mut held = f.w.db.pool.begin().await.expect("hold the entry");
    sqlx::query("SELECT id FROM run_resets WHERE id = $1 FOR UPDATE")
        .bind(made.reset.id.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the entry");
    let (one, two) = (completer(&f), completer(&f));
    let (a, b, ()) = tokio::join!(
        async { one.step(&due(&f, made.reset.id), None).await },
        async { two.step(&due(&f, made.reset.id), None).await },
        async {
            wait_for_backends_on_locks(&f, 2, "both engines wait for the entry").await;
            held.commit().await.expect("release the entry");
        }
    );
    let outcomes = [a.expect("one step"), b.expect("the other step")];
    let failed = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Failed("AUTHORITY_REVOKED")))
        .count();
    let refused = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Refused(reason) if reason.contains("completed")))
        .count();
    assert_eq!((failed, refused), (1, 1), "{outcomes:?}");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.reset' AND resource_id = $1 \
           AND reason LIKE '%failed%'",
    )
    .bind(open.run.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the audit trail");
    assert_eq!(audited, 1, "the failure is audited once");
    assert!(made_from(&f, open.run).await.is_empty());
}

// ------------------------------------------------------------------------------ the failure of an entry (review, N)

#[tokio::test]
async fn a_failure_that_dies_at_any_write_leaves_the_entry_pending_and_sent_again_fails_it_once() {
    let f = fixture().await;
    let session = worker(&f).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // a pending entry whose source has ended and whose requester has since left: the pass means to fail it
    async fn abandoned(f: &Fixture, session: &Session, name: &str) -> (RunId, ResetId) {
        let operator =
            f.w.member(name, sagat_domain::identity::Role::Operator)
                .await;
        let open = open_pair(f, session).await;
        let made =
            f.w.runs
                .reset(
                    &operator,
                    open.run,
                    asking(open.after_a),
                    &format!("failure-key-{name}"),
                    &ctx(),
                )
                .await
                .expect("an operator may reset");
        finish(f, session, &open).await;
        sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = $2")
            .bind(operator.tenant_id().as_uuid())
            .bind(operator.subject().as_str())
            .execute(&f.w.db.pool)
            .await
            .expect("remove the member");
        (open.run, made.reset.id)
    }
    async fn failures(f: &Fixture, id: ResetId) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'run.reset' AND reason = $1",
        )
        .bind(format!("completes reset {id}: failed AUTHORITY_REVOKED"))
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the audit trail")
    }

    let (_, warm) = abandoned(&f, &session, "fayw").await;
    assert_eq!(
        completer(&f)
            .step(&due(&f, warm), None)
            .await
            .expect("warm-up"),
        Completed::Failed("AUTHORITY_REVOKED")
    );

    let (_, probe) = abandoned(&f, &session, "fayp").await;
    wire.watch().await;
    assert_eq!(
        completer(&f)
            .step(&due(&f, probe), None)
            .await
            .expect("the probe"),
        Completed::Failed("AUTHORITY_REVOKED")
    );
    wire.disarm().await;
    let writes = wire.writes().await;
    assert_eq!(
        writes
            .iter()
            .map(|(xid, _, _)| *xid)
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "a failure is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in ["run_resets", "audit_records"] {
        assert!(
            tables.contains(table),
            "a failure is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "a failure makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let (source, id) = abandoned(&f, &session, &format!("fay{position}")).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = completer(&f).step(&due(&f, id), None).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the step answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        let state: (String, Option<String>) =
            sqlx::query_as("SELECT state, failure_reason FROM run_resets WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the entry");
        assert_eq!(
            state,
            ("PENDING_SOURCE".to_owned(), None),
            "write {position}"
        );
        assert_eq!(failures(&f, id).await, 0, "write {position}");

        // sent again: failed, once, with one record
        assert_eq!(
            completer(&f)
                .step(&due(&f, id), None)
                .await
                .unwrap_or_else(|error| panic!(
                    "write {position} ({table}), sent again: {error:?}"
                )),
            Completed::Failed("AUTHORITY_REVOKED"),
            "write {position}"
        );
        let state: (String, Option<String>) =
            sqlx::query_as("SELECT state, failure_reason FROM run_resets WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the entry");
        assert_eq!(
            state,
            ("FAILED".to_owned(), Some("AUTHORITY_REVOKED".to_owned())),
            "write {position}"
        );
        assert_eq!(failures(&f, id).await, 1, "write {position}");
        assert!(made_from(&f, source).await.is_empty(), "write {position}");

        // and a third time finds nothing to do
        assert_eq!(
            completer(&f)
                .step(&due(&f, id), None)
                .await
                .expect("a third step"),
            Completed::Idle,
            "write {position}"
        );
        assert_eq!(failures(&f, id).await, 1, "write {position}");
    }
}

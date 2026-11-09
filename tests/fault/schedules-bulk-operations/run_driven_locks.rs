//! Run-driven transitions never touch the tables of schedules, batch jobs, business identifiers and
//! pending resets (`T018-076`, `SC-018-016`, `FR-018-007`, `FR-018-021`, `FR-018-030`).
//!
//! The aggregates (a schedule, a batch job, the binding of a business identifier, the entry of a
//! reset that waits for its source) learn that a run ended by looking at it, in a transaction of their
//! own; nothing a run does takes one of their rows (ADR-0050 decision 7), so a run-driven transaction
//! cannot be part of a lock cycle with them. The tests hold an exclusive lock on every one of those
//! tables, which stops any statement that reads or writes them, and drive every transition a run goes
//! through without a person asking for it (a worker completes or fails an attempt, a timer fires, a
//! wait runs out, a lease runs out, a deadline passes, a segment hands off to the next) and one a
//! person does ask for (a cancellation): each has to finish while the locks are held. The same locks
//! do stop what belongs to an aggregate (the firing of a schedule, the claim of an identifier, the
//! request and the completion of a pending reset), each seen waiting on its own table in `pg_locks`,
//! which is the proof that they would have stopped a run-driven transaction that touched one. Real
//! PostgreSQL, the real services and passes.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use sagat_application::reset_completion::Completed;
use sagat_application::runs::{BusinessStart, ResetState, RunOperation, StartOptions, StartRun};
use sagat_application::schedule_firing::Fired;
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::execution::{RunId, StepStatus};
use sagat_scheduler::SchedulerSettings;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::Duration;

use crate::cancel_source::{completer, due, finish, open_pair, request, room_for_pending, worker};
use crate::support::bounded::within;
use crate::support::engine::{World, activity, chain, ctx, document, wait_node, with};
use crate::support::scale::{scheduler, sleepers};
use crate::support::schedules::{Fixture, fixture, hourly};

/// The stage limit: a transition that has to wait for a lock never finishes, so this is long enough
/// for a loaded machine and short enough to name the stage that waited.
const STAGE: StdDuration = StdDuration::from_secs(60);

/// Every table of the two aggregates, by name, so that one added later is locked too.
async fn aggregate_tables(f: &Fixture) -> Vec<String> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables \
         WHERE schemaname = current_schema() \
           AND (tablename = 'schedules' OR tablename = 'business_ids' OR tablename = 'run_resets' \
                OR tablename LIKE 'schedule\\_%' OR tablename LIKE 'batch\\_%') \
         ORDER BY 1",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the tables");
    for known in [
        "schedules",
        "schedule_revisions",
        "schedule_actions",
        "schedule_backfills",
        "schedule_obligations",
        "batch_jobs",
        "batch_items",
        "schedule_trigger_handover",
        "business_ids",
        "run_resets",
    ] {
        assert!(
            names.iter().any(|name| name == known),
            "{known} is among the tables of the aggregates: {names:?}"
        );
    }
    names
}

/// A transaction that holds every aggregate table exclusively until it ends.
async fn lock(f: &Fixture, tables: &[String]) -> Transaction<'static, Postgres> {
    let mut tx = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
        tables.join(", ")
    )))
    .execute(&mut *tx)
    .await
    .expect("lock the aggregates");
    tx
}

/// Every one of the tables is locked: a statement of its own that would wait is refused after 300 ms.
async fn assert_locked(f: &Fixture, tables: &[String]) {
    for table in tables {
        // a statement of its own that would wait: refused after 300 ms with `lock_not_available`
        let mut probe = f.w.db.pool.begin().await.expect("a transaction");
        sqlx::query("SET LOCAL lock_timeout = '300ms'")
            .execute(&mut *probe)
            .await
            .expect("a short wait");
        let refused = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table}"
        )))
        .fetch_one(&mut *probe)
        .await;
        let code = match &refused {
            Err(sqlx::Error::Database(error)) => error.code().map(|c| c.into_owned()),
            _ => None,
        };
        assert_eq!(
            code.as_deref(),
            Some("55P03"),
            "{table} is locked, so a statement on it waits: {refused:?}"
        );
        drop(probe);
    }
}

/// Waits until a backend waits for a lock on `table` itself (`pg_locks`, not a time), at most two minutes: a
/// control that is named after a table must be seen waiting for that table and not for some other lock.
async fn wait_for_a_backend_on_the_table(f: &Fixture, table: &str, stage: &str) {
    within(stage, StdDuration::from_secs(120), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks \
                 WHERE locktype = 'relation' AND NOT granted AND relation = $1::regclass",
            )
            .bind(table)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("look at the locks");
            if waiting > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// A run that holds a business identifier: what it goes through must not touch the binding either.
async fn bound(w: &World, workflow: sagat_domain::definition::WorkflowId, id: &str) -> RunId {
    bound_input(w, workflow, id, b"{}").await
}

async fn bound_input(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    id: &str,
    input: &[u8],
) -> RunId {
    w.runs
        .start_with_options(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input,
                correlation_id: None,
                deadline: None,
                key: &format!("{id}-start-key"),
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(id).expect("an identifier"),
                    conflict: ConflictPolicy::Fail,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                ..StartOptions::default()
            },
            &ctx(),
        )
        .await
        .expect("start a run under an identifier")
        .run
        .info
        .run_id
}

#[tokio::test]
async fn no_run_driven_transition_touches_a_table_of_a_schedule_or_a_batch_job() {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    let (workflow, _) = w.publish(chain("run-driven-locks")).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let completes = bound(w, workflow, "locks-completes").await;
    let complete_a = w.one_task(&session).await;
    let fails = bound(w, workflow, "locks-fails").await;
    let fail_a = w.one_task(&session).await;
    let lapses = bound(w, workflow, "locks-lapses").await;
    w.one_task(&session).await;
    w.age_lease(lapses, "a").await;
    let late = bound(w, workflow, "locks-late").await;
    w.age_run_deadline(late).await;
    let cancelled = bound(w, workflow, "locks-cancelled").await;
    let sleeper = sleepers(w, "run-driven-timer", 1).await[0];
    // an aggregate that has work waiting for it, for the control below
    let schedule = f.create("held", &hourly(), &json!({}), None).await;
    f.clock.advance(Duration::hours(2));

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;

    // a worker completes an attempt, and then the step it unlocks
    within("a worker completes", STAGE, async {
        w.complete(&session, &complete_a, &json!({}))
            .await
            .expect("a completes");
        // other runs have their first step ready too: take the one this run's second step is
        let b = w.one_task_of(&session, "act.b").await;
        w.complete(&session, &b, &json!({}))
            .await
            .expect("b completes");
    })
    .await;
    assert_eq!(
        w.run_status(completes).await,
        sagat_domain::execution::RunStatus::Succeeded
    );

    // a worker fails an attempt for good
    within("a worker fails", STAGE, async {
        w.fail(&session, &fail_a, false).await.expect("a fails");
    })
    .await;
    assert!(
        w.run_status(fails).await.is_terminal(),
        "the failure ended the run"
    );

    // a person cancels
    within("a cancellation", STAGE, async {
        w.operate_run(cancelled, RunOperation::Cancel).await;
    })
    .await;

    // the passes: a timer fires, a lease that ran out is taken back, a deadline passes
    let passes = scheduler(w, SchedulerSettings::default(), None);
    let (mut expired, mut enforced) = (0, 0);
    within("the scheduler's passes", STAGE, async {
        loop {
            let round = passes.tick().await;
            expired += round.leases_expired;
            enforced += round.deadlines_enforced;
            if !round.did_work() {
                break;
            }
        }
    })
    .await;
    assert!(expired >= 1, "the lease that ran out was taken back");
    assert_eq!(
        w.step_status(sleeper, "after").await,
        Some(StepStatus::Ready),
        "the timer fired and the step behind it is ready"
    );
    let ended = w.run_status(late).await;
    assert!(
        enforced >= 1
            && (ended.is_terminal() || ended == sagat_domain::execution::RunStatus::Cancelling),
        "the deadline stopped the run: enforced {enforced} times, the run is {ended:?}"
    );

    // --- the control: what belongs to an aggregate does wait for the same locks -----------------
    let firing = {
        let f = std::sync::Arc::new(f);
        let task = tokio::spawn({
            let f = std::sync::Arc::clone(&f);
            let schedule = schedule.clone();
            async move { f.fire(&schedule).await }
        });
        wait_for_a_backend_on_the_table(
            &f,
            "schedules",
            "the firing of a due schedule waits for the row of its schedule",
        )
        .await;
        assert!(
            !task.is_finished(),
            "the firing waits while the locks are held"
        );
        holder.rollback().await.expect("release the locks");
        within("the firing after the locks are released", STAGE, task)
            .await
            .expect("the task")
    };
    assert_eq!(firing, Fired::Acted, "and acts once they are released");
}

/// What a source that a pending reset waits for goes through: the entry holds its place and is not touched by the
/// transition that ends the source, and the entry is what the request and the completion do wait for.
#[tokio::test]
async fn a_source_that_a_pending_reset_waits_for_ends_without_touching_the_register() {
    let f = fixture().await;
    let tables = aggregate_tables(&f).await;
    let session = worker(&f).await;
    room_for_pending(&f, 64).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    // three sources, each with an entry that waits for it (the request cancelled the source, which now ends by what
    // a worker or a pass does): a worker completes the attempt it holds, a worker fails it for good, its lease runs out
    // (the outcome of that attempt is then unknown, the source goes to review and its entry keeps waiting)
    let by_completion = open_pair(&f, &session).await;
    let by_failure = open_pair(&f, &session).await;
    let by_lapse = open_pair(&f, &session).await;
    let mut entries = Vec::new();
    for (open, key) in [
        (&by_completion, "locks-reset-completion"),
        (&by_failure, "locks-reset-failure"),
        (&by_lapse, "locks-reset-lapse"),
    ] {
        let made = request(&f, open.run, open.after_a, key)
            .await
            .expect("the request");
        assert_eq!(made.reset.state, ResetState::PendingSource);
        entries.push(made.reset.id);
    }
    f.w.age_lease(by_lapse.run, "b").await;
    // two more sources for the controls: the request of an entry, and the completion of one
    let for_a_request = open_pair(&f, &session).await;
    let for_a_completion = open_pair(&f, &session).await;
    let waiting = request(
        &f,
        for_a_completion.run,
        for_a_completion.after_a,
        "locks-reset-control",
    )
    .await
    .expect("the request")
    .reset
    .id;
    finish(&f, &session, &for_a_completion).await;

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;

    within("a worker completes a source with an entry", STAGE, async {
        finish(&f, &session, &by_completion).await;
    })
    .await;
    within("a worker fails a source with an entry", STAGE, async {
        f.w.fail(&session, &by_failure.b, false)
            .await
            .expect("b fails");
    })
    .await;
    assert!(
        f.w.run_status(by_failure.run).await.is_terminal(),
        "the failure ended the source"
    );
    let passes = scheduler(&f.w, SchedulerSettings::default(), None);
    let mut expired = 0;
    within(
        "the lease of a source with an entry runs out",
        STAGE,
        async {
            loop {
                let round = passes.tick().await;
                expired += round.leases_expired;
                if !round.did_work() {
                    break;
                }
            }
        },
    )
    .await;
    assert!(expired >= 1, "the lease that ran out was taken back");
    assert_eq!(
        f.w.run_status(by_lapse.run).await,
        sagat_domain::execution::RunStatus::ManualReview,
        "a cancelled source whose attempt has an unknown outcome waits for a person: the entry waits too"
    );

    // --- the control: the completion of an entry waits for the register ---------------------------
    let completion = tokio::spawn({
        let completer = completer(&f);
        let due = due(&f, waiting);
        async move { completer.step(&due, None).await }
    });
    wait_for_a_backend_on_the_table(
        &f,
        "run_resets",
        "the completion of an entry waits for the register",
    )
    .await;
    assert!(
        !completion.is_finished(),
        "the completion waits while the register is locked"
    );
    holder.rollback().await.expect("release the locks");
    let done = within(
        "the completion once the locks are released",
        STAGE,
        completion,
    )
    .await
    .expect("the task")
    .expect("the step");
    assert_eq!(done, Completed::Created, "and acts once they are released");

    // --- the control: the request of an entry waits for the register ------------------------------
    let holder = lock(&f, &tables).await;
    let asking = tokio::spawn({
        let w = std::sync::Arc::clone(&f.w);
        let run = for_a_request.run;
        let sequence = for_a_request.after_a;
        async move {
            w.runs
                .reset(
                    &w.admin,
                    run,
                    crate::cancel_source::asking(sequence),
                    "locks-reset-request-control",
                    &ctx(),
                )
                .await
        }
    });
    wait_for_a_backend_on_the_table(
        &f,
        "run_resets",
        "the request of an entry waits for the register",
    )
    .await;
    assert!(
        !asking.is_finished(),
        "the request waits while the register is locked"
    );
    holder.rollback().await.expect("release the locks");
    let made = within("the request once the locks are released", STAGE, asking)
        .await
        .expect("the task")
        .expect("the request");
    assert_eq!(made.reset.state, ResetState::PendingSource);

    // the entries of the three sources were never moved by what ended them: each still waits, and the completion of
    // those whose source ended makes the run; the one whose source waits for a person is left waiting
    for (entry, expected) in
        entries
            .into_iter()
            .zip([Completed::Created, Completed::Created, Completed::Idle])
    {
        let state: String = sqlx::query_scalar("SELECT state FROM run_resets WHERE id = $1")
            .bind(entry.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the entry");
        assert_eq!(
            state, "PENDING_SOURCE",
            "what ended the source did not move its entry"
        );
        assert_eq!(
            completer(&f)
                .step(&due(&f, entry), None)
                .await
                .expect("the step"),
            expected
        );
    }
}

/// A start under an identifier claims its binding in the creation transaction, which is the one place that touches the
/// table: the claim waits for the same locks that every transition of a run goes through without waiting.
#[tokio::test]
async fn the_claim_of_a_business_identifier_waits_for_the_binding_table_and_acts_once_it_is_free() {
    let f = fixture().await;
    let tables = aggregate_tables(&f).await;
    let holder = lock(&f, &tables).await;
    let claim = tokio::spawn({
        let w = std::sync::Arc::clone(&f.w);
        let workflow = f.workflow;
        async move { bound(&w, workflow, "locks-claim").await }
    });
    wait_for_a_backend_on_the_table(
        &f,
        "business_ids",
        "the claim of an identifier waits for the bindings",
    )
    .await;
    assert!(
        !claim.is_finished(),
        "the claim waits while the table is locked"
    );
    holder.rollback().await.expect("release the locks");
    let run = within("the claim once the locks are released", STAGE, claim)
        .await
        .expect("the task");
    let bound_to: String = sqlx::query_scalar("SELECT execution_id::text FROM business_ids")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the binding");
    assert_eq!(bound_to, run.as_uuid().to_string(), "and binds it once");
}

/// A scheduler that owns the shards, as the hand-off of a chain needs; reused across ticks, since a
/// new placement per tick would starve the later ones.
pub(crate) async fn shard_owner(f: &Fixture) -> crate::support::engine::Sched {
    let placement = sagat_scheduler::Placement::new(format!("run-driven-{}", uuid::Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire the shards");
    scheduler(&f.w, SchedulerSettings::default(), Some(placement))
}

/// A workflow that waits for a signal and, if it does not come, takes the other way.
fn waiter(name: &str) -> serde_json::Value {
    document(
        name,
        "w",
        vec![
            with(
                wait_node("w", "payment.confirmed", "input.k", 300_000, Some("review")),
                "next",
                json!("notify"),
            ),
            activity("notify"),
            activity("review"),
        ],
    )
}

/// A workflow of two segments: one activity, then a hand-off to the next segment.
fn rollover(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 2, "timeoutMs": 3_600_000}
    })
}

#[tokio::test]
async fn a_signal_a_wait_that_times_out_and_a_handoff_take_no_lock_of_an_aggregate() {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;

    // --- everything a transition needs is prepared before the locks are taken -----------------
    let (waiting, _) = w.publish(waiter("run-driven-signals")).await;
    let (rolling, _) = w.publish(rollover("run-driven-handoff")).await;
    let signalled = bound_input(w, waiting, "locks-signalled", br#"{"k": "s"}"#).await;
    let timed_out = bound_input(w, waiting, "locks-timed-out", br#"{"k": "t"}"#).await;
    w.age_due(timed_out, "w").await;
    let chained = bound_input(w, rolling, "locks-chained", br#"{"next": {"segment": 2}}"#).await;
    let session = w
        .session(&["act.notify", "act.review", "segment.work"])
        .await;
    let segment = w.one_task_of(&session, "segment.work").await;
    let owner = shard_owner(&f).await;

    // --- the locks ------------------------------------------------------------------------------
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;

    // a signal wakes the run that waits for it
    let woken = within("a signal", STAGE, async {
        w.runs
            .signal(
                &w.admin,
                signalled,
                "payment.confirmed",
                br#"{"ok": true}"#,
                "signal-under-locks",
                &ctx(),
            )
            .await
            .expect("the signal")
    })
    .await;
    assert_eq!(woken.iter().map(|n| n.as_str()).collect::<Vec<_>>(), ["w"]);

    // the wait of the other one runs out, and it takes the way for that
    let passes = scheduler(w, SchedulerSettings::default(), None);
    within("the wait that times out", STAGE, async {
        for _ in 0..8 {
            if !passes.tick().await.did_work() {
                break;
            }
        }
    })
    .await;
    assert_eq!(
        w.step_status(timed_out, "review").await,
        Some(StepStatus::Ready),
        "the wait ran out and the way for that is ready"
    );

    // a segment ends and the next one starts, under the same identifier
    within("the hand-off", STAGE, async {
        w.complete(&session, &segment, &json!({}))
            .await
            .expect("the segment works");
        for _ in 0..12 {
            let report = owner.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            let count: i32 = sqlx::query_scalar(
                "SELECT segment_count FROM execution_chains WHERE chain_id = $1",
            )
            .bind(chained.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the chain");
            if count >= 2 {
                break;
            }
        }
    })
    .await;
    let segments: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE business_id = $1")
        .bind("locks-chained")
        .fetch_one(&w.db.pool)
        .await
        .expect("segments");
    assert_eq!(segments, 2, "the successor carries the identifier");

    holder.rollback().await.expect("release the locks");
}
// --- the runs the aggregates name ------------------------------------------------------------------------------------------

/// The three kinds of run an aggregate names: a run a schedule started (`runs.schedule_id`), the target of an item of a batch job
/// (a row of `batch_items` names it), and the run a `CANCEL_OTHER` schedule owes a cancellation (a row of `schedule_obligations`).
/// A read of an aggregate table that a run-driven transition made only for such a run would not run for the runs of the other
/// tests, which belong to no aggregate.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Schedule,
    BatchItem,
    Debt,
}

/// A workflow of one timer, then an activity.
fn timer_document(name: &str) -> serde_json::Value {
    document(
        name,
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 3_600_000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    )
}

/// A run of `kind` of the workflow of the fixture, with the aggregate that names it, and whether `schedule` is the schedule that
/// started the runs of this kind (a `Schedule`'s runs share one, which allows them all to be open at once).
async fn named_run(
    f: &Fixture,
    kind: Kind,
    n: usize,
    schedule: &mut Option<sagat_application::schedules::ScheduleRecord>,
    target: Option<serde_json::Value>,
) -> RunId {
    match kind {
        Kind::Schedule => {
            let record = match schedule {
                Some(record) => record.clone(),
                None => {
                    let record = f
                        .create(
                            &format!("run-driven-{n}"),
                            &hourly(),
                            &json!({"overlap": "ALLOW_ALL"}),
                            target,
                        )
                        .await;
                    *schedule = Some(record.clone());
                    record
                }
            };
            f.clock.advance(Duration::hours(1) + Duration::seconds(1));
            assert!(f.drain(&record).await >= 1, "the schedule started a run");
            RunId::new(*f.runs(&record).await.last().expect("a run"))
        }
        Kind::BatchItem => {
            f.w.start(
                target
                    .as_ref()
                    .and_then(|t| t["workflowId"].as_str())
                    .map_or(f.workflow, |id| id.parse().expect("a workflow id")),
                &json!({}),
            )
            .await
        }
        Kind::Debt => {
            let (_, run) = crate::cancel_other::waiting_behind_an_open_run_of(
                f,
                &format!("run-driven-debt-{n}"),
                crate::cancel_other::cancel_other(),
                target,
            )
            .await;
            RunId::new(run)
        }
    }
}

async fn transitions_of_the_runs_named_by(kind: Kind) {
    let f = fixture().await;
    let w = &f.w;
    let tables = aggregate_tables(&f).await;
    let (timer_workflow, _) = w
        .publish(timer_document(&format!(
            "run-driven-timer-{}",
            format!("{kind:?}").to_lowercase()
        )))
        .await;
    let session = w.session(&["act.a", "act.b", "act.after"]).await;
    let timer_target = Some(json!({"workflowId": timer_workflow.to_string()}));

    // --- everything a transition needs is prepared before the locks are taken: the runs that are claimed first, one at a time,
    // because the next task is the only one that is ready; then the runs nobody claims
    let mut schedule = None;
    let completes = named_run(&f, kind, 1, &mut schedule, None).await;
    let complete_a = w.one_task(&session).await;
    let fails = named_run(&f, kind, 2, &mut schedule, None).await;
    let fail_a = w.one_task(&session).await;
    let lapses = named_run(&f, kind, 3, &mut schedule, None).await;
    w.one_task(&session).await;
    w.age_lease(lapses, "a").await;
    let late = named_run(&f, kind, 4, &mut schedule, None).await;
    w.age_run_deadline(late).await;
    let cancelled = named_run(&f, kind, 5, &mut schedule, None).await;
    // the timer: a run of the same kind of a workflow of one timer (another schedule for a schedule's, the same job for an item's)
    let mut timer_schedule = None;
    let sleeper = named_run(&f, kind, 6, &mut timer_schedule, timer_target).await;
    w.age_due(sleeper, "pause").await;
    if matches!(kind, Kind::BatchItem) {
        // the items of a job name every one of these runs; they are selected and not applied, so each is a row that waits
        let runs: Vec<uuid::Uuid> = [completes, fails, lapses, late, cancelled, sleeper]
            .iter()
            .map(|run| run.as_uuid())
            .collect();
        let job = crate::batch::create(&f, &f.w.admin, &runs, "run-driven-batch-key").await;
        let applies = crate::batch::Applies::default();
        let exec = crate::batch::executor(&f, &applies);
        crate::batch::until_it_acts(&f, &exec, &job)
            .await
            .expect("the selection round");
        let named: i64 = sqlx::query_scalar("SELECT count(*) FROM batch_items WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the items");
        assert_eq!(named, 6, "an item names each of the runs");
    }
    let debts: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_obligations")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the debts");
    assert_eq!(
        debts,
        if matches!(kind, Kind::Debt) { 6 } else { 0 },
        "a debt names each run that a CANCEL_OTHER schedule owes a cancellation, and only those"
    );

    // --- the locks
    let holder = lock(&f, &tables).await;
    assert_locked(&f, &tables).await;
    let stage = |what: &str| format!("{kind:?}: {what}");

    within(&stage("a worker completes"), STAGE, async {
        w.complete(&session, &complete_a, &json!({}))
            .await
            .expect("a completes");
        let b = w.one_task_of(&session, "act.b").await;
        w.complete(&session, &b, &json!({}))
            .await
            .expect("b completes");
    })
    .await;
    assert_eq!(
        w.run_status(completes).await,
        sagat_domain::execution::RunStatus::Succeeded
    );
    within(&stage("a worker fails"), STAGE, async {
        w.fail(&session, &fail_a, false).await.expect("a fails");
    })
    .await;
    assert!(
        w.run_status(fails).await.is_terminal(),
        "the failure ended the run"
    );
    within(&stage("a cancellation"), STAGE, async {
        w.operate_run(cancelled, RunOperation::Cancel).await;
    })
    .await;
    let passes = scheduler(w, SchedulerSettings::default(), None);
    let (mut expired, mut enforced) = (0, 0);
    within(&stage("the scheduler's passes"), STAGE, async {
        loop {
            let round = passes.tick().await;
            expired += round.leases_expired;
            enforced += round.deadlines_enforced;
            if !round.did_work() {
                break;
            }
        }
    })
    .await;
    assert!(expired >= 1, "the lease that ran out was taken back");
    assert_eq!(
        w.step_status(sleeper, "after").await,
        Some(StepStatus::Ready),
        "the timer fired and the step behind it is ready"
    );
    let ended = w.run_status(late).await;
    assert!(
        enforced >= 1
            && (ended.is_terminal() || ended == sagat_domain::execution::RunStatus::Cancelling),
        "the deadline stopped the run: enforced {enforced} times, the run is {ended:?}"
    );

    holder.rollback().await.expect("release the locks");
}

#[tokio::test]
async fn the_runs_a_schedule_started_end_without_touching_the_schedule_tables() {
    transitions_of_the_runs_named_by(Kind::Schedule).await;
}

#[tokio::test]
async fn the_target_of_an_item_of_a_batch_job_ends_without_touching_the_batch_tables() {
    transitions_of_the_runs_named_by(Kind::BatchItem).await;
}

#[tokio::test]
async fn the_run_a_cancel_other_schedule_owes_a_cancellation_ends_without_touching_the_debt() {
    transitions_of_the_runs_named_by(Kind::Debt).await;
}

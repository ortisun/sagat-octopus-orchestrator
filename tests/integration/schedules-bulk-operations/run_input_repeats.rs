//! Design 043 against a real PostgreSQL, rows 4 and 5 of the creators through their own entry points: a batch `RERUN`
//! item, a synchronous reset and a pending reset (T043-5, T043-21, FR-043-001, FR-043-003, FR-043-005, FR-043-010,
//! FR-043-011). The sources are admitted before the check was on (a gate in `report` mode), so their stored input is
//! one their version refuses today. The final refusals use the platform's validator; the transient ones a validator
//! double that answers "no room" once and then accepts.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest,
};
use sagat_application::definitions::InputValidationMode;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::reset_completion::{
    Completed, Completion, ResetCompleter, ResetCompletions,
};
use sagat_application::run_input::test_support::Scripted;
use sagat_application::run_input::{RunInputGate, SchemaRefusal};
use sagat_application::runs::{PendingReset, RerunBusinessId, ResetRequest, ResetSourcePolicy};
use sagat_application::schedules::ShardFence;
use sagat_domain::batch::{Action, JobState, Selector};
use sagat_domain::execution::{ResetReapply, RunId};
use sagat_domain::identity::Principal;
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore, PgRunStore};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

use crate::batch_executor::{create, items, job, live, select, settle, write};
use crate::cancel_source::{
    Open, WHY, asking, completed_steps, due_of, end, entries, made_from, outcome,
};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{Runs, activity, ctx, document, with};
use crate::support::run_input::{
    order_schema, publish_in, start_in_report, unusable_schema, with_schema,
};
use crate::support::schedules::TestClock;

/// A run service whose gate asks a validator double that gives `answers`, one per call, then accepts.
fn gated(f: &Fixture, answers: Vec<Result<(), SchemaRefusal>>) -> Runs {
    sagat_application::runs::RunService::new(
        f.w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_run_inputs(Arc::new(RunInputGate::new(
        Arc::new(Scripted::answering(answers)),
        InputValidationMode::Enforce,
    )))
    .with_declared_updates(Arc::new(sagat_infra_ir::JsonDeclaredOperationEvaluator))
    .with_reset_reapply(true)
    .with_reset_cancel_source(true)
}

/// A run service as the world's, with the platform's validator in `enforce`.
fn enforcing(f: &Fixture) -> Runs {
    sagat_application::runs::RunService::new(
        f.w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Enforce,
    ))
    .with_declared_updates(Arc::new(sagat_infra_ir::JsonDeclaredOperationEvaluator))
    .with_reset_reapply(true)
    .with_reset_cancel_source(true)
}

/// A run service whose gate reports, as a control plane in `report` mode (or one of 0.2.x) admits a request.
fn reporting(f: &Fixture) -> Runs {
    sagat_application::runs::RunService::new(
        f.w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Report,
    ))
    .with_declared_updates(Arc::new(sagat_infra_ir::JsonDeclaredOperationEvaluator))
    .with_reset_reapply(true)
    .with_reset_cancel_source(true)
}

/// The two transient answers of the validator: no permit within the wait, and the backstop.
fn transient() -> [SchemaRefusal; 2] {
    [SchemaRefusal::Busy, SchemaRefusal::Backstop]
}

/// A chain `a -> b -> c` whose version has `schema`, started before the check was on with `{"k": 1}`; `a` is done and
/// `b` is leased, so the source has not ended.
async fn open_admitted_in_report(f: &Fixture, schema: Value, mode: InputValidationMode) -> Open {
    let tag = Uuid::now_v7().simple().to_string();
    let (a, b, c) = (format!("a{tag}"), format!("b{tag}"), format!("c{tag}"));
    let doc = with_schema(
        document(
            &format!("repeats-{tag}"),
            &a,
            vec![
                with(activity(&a), "next", json!(b)),
                with(activity(&b), "next", json!(c)),
                activity(&c),
            ],
        ),
        schema,
    );
    let (workflow, _) = publish_in(&f.w, mode, &doc).await;
    let run = start_in_report(&f.w, workflow, &json!({"k": 1})).await;
    let names = [format!("act.{a}"), format!("act.{b}"), format!("act.{c}")];
    let session =
        f.w.session(&names.iter().map(String::as_str).collect::<Vec<_>>())
            .await;
    let first = f.w.one_task_of(&session, &names[0]).await;
    f.w.complete(&session, &first, &json!({"done": "a"}))
        .await
        .expect("a completes");
    let second = f.w.one_task_of(&session, &names[1]).await;
    let after_a = completed_steps(f, run).await[0];
    Open {
        run,
        session,
        b: second,
        after_a,
    }
}

/// As [`open_admitted_in_report`], then ended: the attempt of `b` completes after a cancellation.
async fn ended_admitted_in_report(f: &Fixture, schema: Value, mode: InputValidationMode) -> Open {
    let open = open_admitted_in_report(f, schema, mode).await;
    f.w.operate_run(open.run, sagat_application::runs::RunOperation::Cancel)
        .await;
    end(f, &open).await;
    open
}

fn of_an_ended_source(sequence: u64) -> ResetRequest<'static> {
    ResetRequest {
        sequence,
        reason: WHY,
        acknowledge_reexecution: true,
        business: RerunBusinessId::Keep,
        reapply: ResetReapply::None,
        source_policy: ResetSourcePolicy::RequireTerminal,
    }
}

fn reset_key() -> String {
    format!("reset-{}", Uuid::now_v7())
}

/// Row 5, synchronous: a reset of an ended run whose stored input its version refuses is refused `RUN_INPUT_INVALID`
/// with `/` and `other` (a repeat names no property of a payload the caller may not see, D043-121), and creates no run
/// and no entry of the register.
#[tokio::test]
async fn row_5_a_synchronous_reset_of_an_input_the_version_refuses_says_the_root_and_other() {
    let f = fixture().await;
    let open = ended_admitted_in_report(&f, order_schema(), InputValidationMode::Enforce).await;
    let refused =
        f.w.runs
            .reset(
                &f.w.admin,
                open.run,
                of_an_ended_source(open.after_a),
                &reset_key(),
                &ctx(),
            )
            .await
            .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/".to_owned()),
            rule: Some("other"),
        }
    );
    assert!(made_from(&f, open.run).await.is_empty(), "no run");
    assert_eq!(entries(&f, open.run).await, 0, "no entry");
}

/// Row 5, a schema outside the subset (stored before publication refused it): the reset is refused
/// `RUN_INPUT_SCHEMA_UNUSABLE` with neither a field nor a rule (OD-2026-11-14-11), and creates nothing.
#[tokio::test]
async fn row_5_a_reset_of_a_version_whose_schema_is_unusable_is_refused_schema_unusable() {
    let f = fixture().await;
    let open = ended_admitted_in_report(&f, unusable_schema(), InputValidationMode::Report).await;
    let refused =
        f.w.runs
            .reset(
                &f.w.admin,
                open.run,
                of_an_ended_source(open.after_a),
                &reset_key(),
                &ctx(),
            )
            .await
            .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_SCHEMA_UNUSABLE",
            field: None,
            rule: None,
        }
    );
    assert!(made_from(&f, open.run).await.is_empty(), "no run");
    assert_eq!(entries(&f, open.run).await, 0, "no entry");
}

/// T043-21 for a synchronous reset: no room in the validator (no permit within the wait, or the backstop) is
/// `Unavailable` (a `503`), never final; the same request then makes the run once there is room.
#[tokio::test]
async fn a_synchronous_reset_with_no_room_is_unavailable_and_the_retry_makes_the_run() {
    let f = fixture().await;
    for refusal in transient() {
        let open =
            ended_admitted_in_report(&f, json!({"type": "object"}), InputValidationMode::Enforce)
                .await;
        let runs = gated(&f, vec![Err(refusal.clone())]);
        let key = reset_key();
        let first = runs
            .reset(
                &f.w.admin,
                open.run,
                of_an_ended_source(open.after_a),
                &key,
                &ctx(),
            )
            .await
            .expect_err("no room");
        assert_eq!(first, AppError::Unavailable, "{refusal:?}");
        assert!(
            made_from(&f, open.run).await.is_empty(),
            "{refusal:?}: no run yet"
        );
        runs.reset(
            &f.w.admin,
            open.run,
            of_an_ended_source(open.after_a),
            &key,
            &ctx(),
        )
        .await
        .expect("the retry with room");
        assert_eq!(
            made_from(&f, open.run).await.len(),
            1,
            "{refusal:?}: one run"
        );
    }
}

/// The completer of the pending resets, completing through `runs`.
struct Completions(Runs);

impl ResetCompletions for Completions {
    async fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        self.0.complete_reset(principal, entry, fence, ctx).await
    }
}

type Completer =
    ResetCompleter<PgRunStore, Completions, PgIdentityStore, sagat_application::clock::SystemClock>;

fn completer_with(f: &Fixture, runs: Runs) -> Completer {
    ResetCompleter::new(
        f.w.store.clone(),
        Completions(runs),
        PgIdentityStore::new(f.w.db.pool.clone()),
        sagat_application::clock::SystemClock,
    )
}

/// Asks a `CANCEL_SOURCE` reset of the open source and ends the source: the entry is pending and due.
async fn pending_of(f: &Fixture, open: &Open) -> sagat_domain::execution::ResetId {
    pending_through(f, &f.w.runs, open).await
}

/// As [`pending_of`], the request answered by `runs`.
async fn pending_through(
    f: &Fixture,
    runs: &Runs,
    open: &Open,
) -> sagat_domain::execution::ResetId {
    let made = runs
        .reset(
            &f.w.admin,
            open.run,
            asking(open.after_a, true),
            &reset_key(),
            &ctx(),
        )
        .await
        .expect("the request");
    end(f, open).await;
    made.reset.id
}

/// Row 5, pending, correctness review of 2026-11-16, Major 2: a `CANCEL_SOURCE` reset whose source's input the version
/// refuses is refused at its request (`/` and `other`; `RUN_INPUT_SCHEMA_UNUSABLE` for a schema outside the subset),
/// before it cancels anything: the source goes on running and no entry is written. The version and the input are fixed
/// when the reset is asked, so the outcome is known then.
#[tokio::test]
async fn row_5_a_cancelling_reset_whose_input_the_version_refuses_is_refused_and_cancels_nothing() {
    let f = fixture().await;
    for (schema, mode, reason) in [
        (
            order_schema(),
            InputValidationMode::Enforce,
            "RUN_INPUT_INVALID",
        ),
        (
            unusable_schema(),
            InputValidationMode::Report,
            "RUN_INPUT_SCHEMA_UNUSABLE",
        ),
    ] {
        let open = open_admitted_in_report(&f, schema, mode).await;
        let refused =
            f.w.runs
                .reset(
                    &f.w.admin,
                    open.run,
                    asking(open.after_a, true),
                    &reset_key(),
                    &ctx(),
                )
                .await
                .map(|_| ())
                .expect_err("refused at the request");
        let expected = if reason == "RUN_INPUT_INVALID" {
            AppError::InvalidRunInput {
                reason,
                field: Some("/".to_owned()),
                rule: Some("other"),
            }
        } else {
            AppError::InvalidRunInput {
                reason,
                field: None,
                rule: None,
            }
        };
        assert_eq!(refused, expected);
        assert_eq!(
            f.w.run_status(open.run).await,
            sagat_domain::execution::RunStatus::Running,
            "the source is not cancelled"
        );
        assert_eq!(entries(&f, open.run).await, 0, "no entry");
    }
}

/// Row 5, pending: the engine completing a pending reset whose source's input the version refuses (the request admitted
/// by a control plane in `report` mode, or of 0.2.x) ends the entry `ADMISSION_REFUSED`, final, with no run; a schema
/// outside the subset ends it the same way. The completion stays the authority.
#[tokio::test]
async fn row_5_a_pending_reset_whose_input_the_version_refuses_ends_admission_refused() {
    let f = fixture().await;
    for (schema, mode) in [
        (order_schema(), InputValidationMode::Enforce),
        (unusable_schema(), InputValidationMode::Report),
    ] {
        let open = open_admitted_in_report(&f, schema, mode).await;
        let id = pending_through(&f, &reporting(&f), &open).await;
        let completer = completer_with(&f, enforcing(&f));
        assert_eq!(
            completer
                .step(&due_of(&f, id), None)
                .await
                .expect("the step"),
            Completed::Failed("ADMISSION_REFUSED")
        );
        let (state, reason, new_run, completed) = outcome(&f, id).await;
        assert_eq!(
            (state.as_str(), reason.as_deref(), new_run, completed),
            ("FAILED", Some("ADMISSION_REFUSED"), None, true)
        );
        assert!(made_from(&f, open.run).await.is_empty());
    }
}

/// T043-21 for a pending reset: no room in the validator leaves the entry pending (the pass is told `Unavailable` and
/// tries again), never failed; the next step with room creates the run.
#[tokio::test]
async fn a_pending_reset_with_no_room_is_left_pending_and_the_next_step_creates_the_run() {
    let f = fixture().await;
    for refusal in transient() {
        let open =
            open_admitted_in_report(&f, json!({"type": "object"}), InputValidationMode::Enforce)
                .await;
        let id = pending_of(&f, &open).await;
        let completer = completer_with(&f, gated(&f, vec![Err(refusal.clone())]));
        assert_eq!(
            completer.step(&due_of(&f, id), None).await,
            Err(AppError::Unavailable),
            "{refusal:?}"
        );
        let (state, reason, new_run, completed) = outcome(&f, id).await;
        assert_eq!(
            (state.as_str(), reason, new_run, completed),
            ("PENDING_SOURCE", None, None, false),
            "{refusal:?}: left pending"
        );
        assert_eq!(
            completer.step(&due_of(&f, id), None).await.expect("room"),
            Completed::Created,
            "{refusal:?}"
        );
        assert_eq!(made_from(&f, open.run).await.len(), 1);
    }
}

/// The executor's single-run use cases, through `runs`.
struct Items(Runs);

impl ItemActions for Items {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0, request).await
    }
}

type Executor = BatchExecutor<PgBatchStore, Items, PgIdentityStore, TestClock>;

fn executor_through(f: &Fixture, runs: Runs) -> Executor {
    BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Items(runs),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings::default(),
    )
}

/// An ended run of a version with `schema`, admitted before the check was on with `{"k": 1}`.
async fn ended_source(f: &Fixture, schema: Value, mode: InputValidationMode) -> RunId {
    let (workflow, _) = publish_in(
        &f.w,
        mode,
        &with_schema(
            document(
                &format!("rerun-{}", Uuid::now_v7()),
                "a",
                vec![activity("a")],
            ),
            schema,
        ),
    )
    .await;
    let run = start_in_report(&f.w, workflow, &json!({"k": 1})).await;
    crate::business_ids::cancel(f, run).await;
    run
}

/// Row 4, batch: a `RERUN` item whose source's input the version refuses ends `FAILED` with `VALIDATION_FAILED`, the
/// first time and never retried; so does one whose schema is outside the subset. The job completes.
#[tokio::test]
async fn row_4_a_batch_rerun_item_refused_for_its_input_fails_with_validation_failed() {
    let f = fixture().await;
    let refused = ended_source(&f, order_schema(), InputValidationMode::Enforce).await;
    let unusable = ended_source(&f, unusable_schema(), InputValidationMode::Report).await;
    live(&f);
    let exec = executor_through(&f, enforcing(&f));
    let mut sources = vec![refused.as_uuid(), unusable.as_uuid()];
    sources.sort_unstable();
    let job_ = create(
        &f,
        write(Action::Rerun, Selector::run_ids(sources).expect("selector")),
        &format!("rerun-refused-{}", Uuid::now_v7()),
    )
    .await;
    select(&exec, &job_).await;
    // one item per slot: the second waits for the job's pace
    for _ in 0..12 {
        settle(&exec, &job_).await;
        f.clock.advance(Duration::seconds(10));
    }
    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (
            done.counts.failed,
            done.counts.deferred,
            done.counts.succeeded
        ),
        (2, 0, 0)
    );
    for row in items(&f, job_.id).await {
        assert_eq!(
            (row.1.as_str(), row.3.as_deref()),
            ("FAILED", Some("VALIDATION_FAILED")),
            "{row:?}"
        );
    }
    assert!(made_from(&f, refused).await.is_empty());
    assert!(made_from(&f, unusable).await.is_empty());
}

/// T043-21 for a batch item: no room in the validator defers the item (`LOCK_WAIT`, the class of a transient refusal),
/// never fails it; after the backoff the item succeeds and says which run it made.
#[tokio::test]
async fn a_batch_rerun_item_with_no_room_is_deferred_and_succeeds_once_there_is_room() {
    let f = fixture().await;
    for refusal in transient() {
        let source =
            ended_source(&f, json!({"type": "object"}), InputValidationMode::Enforce).await;
        live(&f);
        let exec = executor_through(&f, gated(&f, vec![Err(refusal.clone())]));
        let job_ = create(
            &f,
            write(
                Action::Rerun,
                Selector::run_ids(vec![source.as_uuid()]).expect("selector"),
            ),
            &format!("rerun-no-room-{}", Uuid::now_v7()),
        )
        .await;
        select(&exec, &job_).await;
        settle(&exec, &job_).await;
        let deferred = job(&f, job_.id).await;
        assert_eq!(
            (
                deferred.counts.deferred,
                deferred.counts.failed,
                deferred.counts.succeeded
            ),
            (1, 0, 0),
            "{refusal:?}"
        );
        let row = &items(&f, job_.id).await[0];
        assert_eq!(
            (row.1.as_str(), row.2.as_deref()),
            ("DEFERRED", Some("LOCK_WAIT")),
            "{refusal:?}"
        );
        f.clock.advance(Duration::minutes(6));
        settle(&exec, &job_).await;
        let done = job(&f, job_.id).await;
        assert_eq!(done.state, JobState::Completed, "{refusal:?}");
        assert_eq!(
            (done.counts.succeeded, done.counts.failed),
            (1, 0),
            "{refusal:?}"
        );
        assert_eq!(made_from(&f, source).await.len(), 1, "{refusal:?}");
    }
}

//! The actions of a batch job against real runs (`T018-047`, `FR-018-030`, `FR-018-031`,
//! `FR-018-034`): each action reaches the single-run use case under the key of its item, and what
//! the use case refuses because the run is not in the state the action needs is a skip with its
//! reason, never a failure. Real PostgreSQL, the real run service and real workers.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use sagat_application::batch::{BatchWrite, JobRecord, NamedPayload};
use sagat_domain::batch::{Action, Filter, JobState, Selector};
use sagat_domain::execution::{RunId, RunStatus};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

use crate::batch_executor::{Executor, create, due, executor, live, select, service, write};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{
    World, activity, document, effect_activities, effect_node, flaky, saga_node, wait_node, with,
};

/// `(run, state, reason, error code, new run)` of an item as the store keeps it.
type Row = (Uuid, String, Option<String>, Option<String>, Option<Uuid>);

/// `(state, reason, error code, new run)` of the item of each run.
type Outcomes = BTreeMap<Uuid, (String, Option<String>, Option<String>, Option<Uuid>)>;

fn over(runs: &[RunId]) -> Selector {
    Selector::run_ids(runs.iter().map(|run| run.as_uuid()).collect()).expect("selector")
}

/// Creates the job and steps it, a second of the test clock at a time, until it has ended.
async fn run_job(f: &Fixture, exec: &Executor, write: BatchWrite, key: &str) -> JobRecord {
    live(f);
    let job = create(f, write, key).await;
    select(exec, &job).await;
    for _ in 0..300 {
        let now = service(f).get(&f.w.admin, job.id).await.expect("job");
        if matches!(
            now.state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return now;
        }
        f.clock.advance(Duration::seconds(1));
        exec.step(&due(&job), None).await.expect("step");
    }
    panic!("the job never ended");
}

async fn outcomes(f: &Fixture, job: &JobRecord) -> Outcomes {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT run_id, state, reason, error_code, new_run_id FROM batch_items WHERE job_id = $1",
    )
    .bind(job.id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("items");
    rows.into_iter()
        .map(|(run, state, reason, error, new_run)| (run, (state, reason, error, new_run)))
        .collect()
}

fn item(outcomes: &Outcomes, run: RunId) -> (&str, Option<&str>, Option<&str>, Option<Uuid>) {
    let (state, reason, error, new_run) = &outcomes[&run.as_uuid()];
    (state, reason.as_deref(), error.as_deref(), *new_run)
}

/// A run of the chain workflow of the fixture that a worker has finished.
async fn finished_run(f: &Fixture) -> RunId {
    let run = f.w.start(f.workflow, &json!({})).await;
    let session = f.w.session(&["act.a", "act.b"]).await;
    for _ in 0..2 {
        let task = f.w.one_task(&session).await;
        f.w.complete(&session, &task, &json!({}))
            .await
            .expect("the step completes");
    }
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);
    run
}

async fn paused(f: &Fixture, run: RunId) -> bool {
    sqlx::query_scalar("SELECT paused FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("paused")
}

fn counts(job: &JobRecord) -> (u32, u32, u32, u32, u32) {
    let c = &job.counts;
    (c.selected, c.succeeded, c.skipped, c.failed, c.cancelled)
}

#[tokio::test]
async fn pause_and_resume_apply_to_the_runs_they_fit_and_skip_the_others() {
    let f = fixture().await;
    let exec = executor(&f);
    let ended = finished_run(&f).await;
    let open = f.w.start(f.workflow, &json!({})).await;
    let both = [ended, open];

    let pause = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&both),
            ..write(
                Action::Pause,
                Selector::filter(Filter::default()).expect("s"),
            )
        },
        "actions-pause",
    )
    .await;
    let done = outcomes(&f, &pause).await;
    assert_eq!(item(&done, open), ("SUCCEEDED", None, None, None));
    assert_eq!(
        item(&done, ended),
        ("SKIPPED", Some("NOT_APPLICABLE"), None, None),
        "a run that is over cannot be paused"
    );
    assert_eq!(counts(&pause), (2, 1, 1, 0, 0));
    assert!(paused(&f, open).await);
    // the audit of the run says which job paused it, and why
    let why: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'run.paused' AND resource_id = $1",
    )
    .bind(open.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the audit of the pause");
    assert!(
        why.as_deref()
            .is_some_and(|reason| reason.starts_with(&format!("batch {}: ", pause.id))),
        "{why:?}"
    );

    let resume = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&both),
            ..write(
                Action::Resume,
                Selector::filter(Filter::default()).expect("s"),
            )
        },
        "actions-resume",
    )
    .await;
    let done = outcomes(&f, &resume).await;
    assert_eq!(item(&done, open), ("SUCCEEDED", None, None, None));
    assert_eq!(
        item(&done, ended),
        ("SKIPPED", Some("NOT_APPLICABLE"), None, None)
    );
    assert!(!paused(&f, open).await);

    // a run that is not paused has nothing to resume
    let again = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&[open]),
            ..write(
                Action::Resume,
                Selector::filter(Filter::default()).expect("s"),
            )
        },
        "actions-resume-again",
    )
    .await;
    assert_eq!(
        item(&outcomes(&f, &again).await, open),
        ("SKIPPED", Some("NOT_APPLICABLE"), None, None)
    );
}

fn payment_flow(name: &str) -> Value {
    document(
        name,
        "w",
        vec![
            with(
                wait_node(
                    "w",
                    "payment.confirmed",
                    "input.paymentId",
                    300_000,
                    Some("review"),
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
            activity("review"),
        ],
    )
}

#[tokio::test]
async fn a_signal_reaches_the_runs_that_wait_for_it_and_is_never_buffered_for_the_others() {
    let f = fixture().await;
    let exec = executor(&f);
    let (payments, _) = f.w.publish(payment_flow("payments")).await;
    let waiting_one = f.w.start(payments, &json!({"paymentId": "p-1"})).await;
    let waiting_two = f.w.start(payments, &json!({"paymentId": "p-2"})).await;
    assert_eq!(f.w.run_status(waiting_one).await, RunStatus::Waiting);
    // a run that waits for no signal at all
    let plain = f.w.start(f.workflow, &json!({})).await;
    let runs = [waiting_one, waiting_two, plain];

    let signal = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&runs),
            signal: Some(NamedPayload {
                name: "payment.confirmed".to_owned(),
                body: br#"{"ok": true, "amount": 150}"#.to_vec(),
            }),
            ..write(
                Action::Signal,
                Selector::filter(Filter::default()).expect("s"),
            )
        },
        "actions-signal",
    )
    .await;
    let done = outcomes(&f, &signal).await;
    assert_eq!(item(&done, waiting_one), ("SUCCEEDED", None, None, None));
    assert_eq!(item(&done, waiting_two), ("SUCCEEDED", None, None, None));
    assert_eq!(
        item(&done, plain),
        ("SKIPPED", Some("NOT_WAITING"), None, None),
        "no step of that run waits for the signal"
    );
    assert_eq!(counts(&signal), (3, 2, 1, 0, 0));
    assert!(
        signal.payload_digest.is_some(),
        "the job shows that it has a payload, by digest"
    );
    assert_eq!(f.w.run_status(waiting_one).await, RunStatus::Running);
    assert_eq!(f.w.run_status(waiting_two).await, RunStatus::Running);

    // the same signal again finds nobody waiting: it is not stored for later
    let again = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&[waiting_one]),
            signal: Some(NamedPayload {
                name: "payment.confirmed".to_owned(),
                body: br#"{"ok": true}"#.to_vec(),
            }),
            ..write(
                Action::Signal,
                Selector::filter(Filter::default()).expect("s"),
            )
        },
        "actions-signal-again",
    )
    .await;
    assert_eq!(
        item(&outcomes(&f, &again).await, waiting_one),
        ("SKIPPED", Some("NOT_WAITING"), None, None)
    );
}

/// A run whose only step lost its worker: the outcome is unknown and the step waits for a person.
async fn run_awaiting_review(w: &World) -> RunId {
    let (flow, _) = w
        .publish(document(
            &format!("charge-{}", Uuid::now_v7().simple()),
            "charge",
            vec![flaky("charge", 3)],
        ))
        .await;
    let run = w.start(flow, &json!({})).await;
    let session = w.session(&["act.charge"]).await;
    let _task = w.one_task(&session).await;
    w.age_lease(run, "charge").await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    run
}

fn step(node: &str, action: Action) -> BatchWrite {
    BatchWrite {
        step_id: Some(node.to_owned()),
        ..write(action, Selector::filter(Filter::default()).expect("s"))
    }
}

/// A run that was cancelled, or failed, after `reserve`: the compensation of `reserve` ran out of
/// attempts and waits for an operator (`reserve~c`), the way a failed compensation does.
async fn run_with_a_stuck_compensation(w: &World) -> RunId {
    let flow = document(
        &format!("settlement-{}", Uuid::now_v7().simple()),
        "settlement",
        vec![
            saga_node("settlement", "reserve", Some("notify")),
            with(
                effect_node("reserve", Some("submit"), false),
                "compensationRetry",
                json!({ "maxAttempts": 1, "initialDelayMs": 100, "maxDelayMs": 100,
                        "strategy": "fixed", "jitter": false }),
            ),
            effect_node("submit", None, true),
            activity("notify"),
        ],
    );
    let (workflow, _) = w.publish(flow).await;
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    let session = w
        .session(&names.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&session).await;
    w.complete(&session, &reserve, &json!({}))
        .await
        .expect("reserve");
    let submit = w.one_task(&session).await;
    w.fail(&session, &submit, false)
        .await
        .expect("submit fails");
    let undo = w.one_task(&session).await;
    w.fail_with(&session, &undo, "PROVIDER_DOWN", true)
        .await
        .expect("the provider is down");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    run
}

#[tokio::test]
async fn retry_step_retries_a_failed_compensation_and_never_repeats_an_unknown_outcome() {
    let f = fixture().await;
    let exec = executor(&f);
    let stuck = run_with_a_stuck_compensation(&f.w).await;
    // a run that has no such step at all
    let other = f.w.start(f.workflow, &json!({})).await;

    let retry = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&[stuck, other]),
            ..step("reserve~c", Action::RetryStep)
        },
        "actions-retry",
    )
    .await;
    let done = outcomes(&f, &retry).await;
    assert_eq!(item(&done, stuck), ("SUCCEEDED", None, None, None));
    assert_eq!(
        item(&done, other),
        ("SKIPPED", Some("NOT_WAITING"), None, None),
        "a run without a step of that name is not for this action, and it is not hidden"
    );
    assert_eq!(counts(&retry), (2, 1, 1, 0, 0));
    assert_eq!(
        f.w.run_status(stuck).await,
        RunStatus::Compensating,
        "the compensation has another attempt to make"
    );

    // a step whose outcome nobody knows must be settled by a person before anything is repeated
    let unknown = run_awaiting_review(&f.w).await;
    let blind = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&[unknown]),
            ..step("charge", Action::RetryStep)
        },
        "actions-retry-unknown",
    )
    .await;
    assert_eq!(
        item(&outcomes(&f, &blind).await, unknown),
        ("FAILED", None, Some("CONFLICT"), None),
        "the item fails, with nothing applied, and the run stays in review"
    );
    assert_eq!(counts(&blind), (1, 0, 0, 1, 0));
    assert_eq!(f.w.run_status(unknown).await, RunStatus::ManualReview);
}

#[tokio::test]
async fn mark_manual_review_holds_a_step_that_has_not_started_and_skips_the_others() {
    let f = fixture().await;
    let exec = executor(&f);
    let (flow, _) =
        f.w.publish(document("held", "charge", vec![activity("charge")]))
            .await;
    let ready = f.w.start(flow, &json!({})).await;
    let other = f.w.start(f.workflow, &json!({})).await;

    let hold = run_job(
        &f,
        &exec,
        BatchWrite {
            selector: over(&[ready, other]),
            ..step("charge", Action::MarkManualReview)
        },
        "actions-hold",
    )
    .await;
    let done = outcomes(&f, &hold).await;
    assert_eq!(item(&done, ready), ("SUCCEEDED", None, None, None));
    assert_eq!(
        item(&done, other),
        ("SKIPPED", Some("NOT_WAITING"), None, None)
    );
    assert_eq!(f.w.run_status(ready).await, RunStatus::ManualReview);
    assert_eq!(counts(&hold), (2, 1, 1, 0, 0));
}

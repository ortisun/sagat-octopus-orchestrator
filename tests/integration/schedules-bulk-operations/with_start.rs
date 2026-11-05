//! Signal-with-Start (`T018-040`, `FR-018-024`, `FR-018-026`, `SC-018-008`): the open run of a business
//! identifier is signalled; otherwise a run is created and the signal is delivered to it in the same
//! transaction, and if no step of it waits for the signal the whole request is rolled back. A signal is
//! never buffered. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::runs::{BusinessStart, SignalAtStart, SignalStarted, StartRun};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunStatus;
use sagat_domain::identity::Role;
use serde_json::json;

use crate::business_ids::{cancel, holder_of, key, runs_of};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{activity, ctx, document, wait_node, with};

const SIGNAL: &str = "payment.confirmed";

/// A workflow whose entry waits for the signal, and then does one activity.
async fn waiter(f: &Fixture) -> WorkflowId {
    f.w.publish(document(
        &format!("with-start-waiter-{}", uuid::Uuid::now_v7()),
        "w",
        vec![
            with(
                wait_node("w", SIGNAL, "input.k", 300_000, None),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    ))
    .await
    .0
}

fn business(id: &str, reuse: ReusePolicy) -> BusinessStart {
    BusinessStart {
        id: BusinessId::new(id).expect("an identifier"),
        // an open holder is always signalled: the policy of a with-start request is not a choice
        conflict: ConflictPolicy::UseExisting,
        reuse,
        window: ReuseWindow::default(),
    }
}

async fn signal_with_start(
    f: &Fixture,
    workflow: WorkflowId,
    id: &str,
    payload: &[u8],
    key: &str,
) -> Result<SignalStarted, AppError> {
    f.w.runs
        .signal_with_start(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: br#"{"k": "s"}"#,
                correlation_id: None,
                deadline: None,
                key,
            },
            business(id, ReusePolicy::AllowDuplicate),
            SignalAtStart {
                name: SIGNAL,
                payload,
            },
            &ctx(),
        )
        .await
}

async fn journal(f: &Fixture, run: sagat_domain::execution::RunId) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
        .bind(run.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the journal")
}

fn nodes(done: &SignalStarted) -> Vec<&str> {
    done.delivered_to.iter().map(|n| n.as_str()).collect()
}

#[tokio::test]
async fn an_absent_identifier_starts_a_run_whose_entry_waits_and_delivers_the_signal_at_creation() {
    let f = fixture().await;
    let waiting = waiter(&f).await;

    let done = signal_with_start(&f, waiting, "cart-42", br#"{"item": "book"}"#, &key())
        .await
        .expect("started and signalled");

    assert!(done.started);
    assert_eq!(nodes(&done), ["w"]);
    let run = done.run.info.run_id;
    assert_eq!(holder_of(&f, "cart-42").await, Some(run.as_uuid()));
    let events = journal(&f, run).await;
    assert_eq!(events.first().map(String::as_str), Some("run.started"));
    assert!(
        events.iter().any(|e| e == "wait.satisfied"),
        "the signal was delivered in the creation: {events:?}"
    );
    assert_eq!(
        f.w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Succeeded)
    );
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn the_open_run_that_waits_for_the_signal_is_signalled_and_nothing_is_started() {
    let f = fixture().await;
    let waiting = waiter(&f).await;
    // the holder: started under the identifier by an ordinary start, so it waits
    let holder =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: waiting,
                    version: None,
                    input: br#"{"k": "s"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                sagat_application::runs::StartOptions {
                    business: Some(business("cart-43", ReusePolicy::AllowDuplicate)),
                    ..sagat_application::runs::StartOptions::default()
                },
                &ctx(),
            )
            .await
            .expect("the holder")
            .run
            .info
            .run_id;
    assert_eq!(f.w.run_status(holder).await, RunStatus::Waiting);

    let done = signal_with_start(&f, waiting, "cart-43", b"{}", &key())
        .await
        .expect("signalled");

    assert!(!done.started);
    assert_eq!(done.run.info.run_id, holder);
    assert_eq!(nodes(&done), ["w"]);
    assert_eq!(runs_of(&f, "cart-43").await.len(), 1, "no second run");
    assert_eq!(
        f.w.step_status(holder, "w").await,
        Some(sagat_domain::execution::StepStatus::Succeeded)
    );
}

#[tokio::test]
async fn an_open_run_that_no_longer_waits_is_not_signalled_and_nothing_changes() {
    let f = fixture().await;
    let waiting = waiter(&f).await;
    let first = signal_with_start(&f, waiting, "cart-44", b"{}", &key())
        .await
        .expect("started and signalled");
    let holder = first.run.info.run_id;
    let before = journal(&f, holder).await;

    // the holder is open (its activity is ready) and waits for nothing
    let refused = signal_with_start(&f, waiting, "cart-44", b"{}", &key()).await;

    assert_eq!(
        refused.err(),
        Some(AppError::SignalRefused {
            reason: "NOT_WAITING"
        })
    );
    assert_eq!(journal(&f, holder).await, before, "nothing changed");
    assert_eq!(runs_of(&f, "cart-44").await.len(), 1);
}

#[tokio::test]
async fn an_entry_that_does_not_wait_for_the_signal_rolls_everything_back() {
    let f = fixture().await;
    // the fixture's own workflow starts with an activity: nothing waits for a signal at creation
    let runs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");

    let refused = signal_with_start(&f, f.workflow, "cart-45", b"{}", &key()).await;

    assert_eq!(
        refused.err(),
        Some(AppError::SignalRefused {
            reason: "SIGNAL_NOT_ACCEPTED_AT_START"
        })
    );
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs_after, runs_before, "no run was created");
    assert_eq!(holder_of(&f, "cart-45").await, None, "and no binding");
    // the identifier is free: an ordinary start may take it
    let free = crate::business_ids::start(
        &f,
        crate::business_ids::business("cart-45", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await;
    assert!(free.is_ok(), "{free:?}");
}

#[tokio::test]
async fn the_reuse_policy_applies_to_a_closed_holder_and_a_signal_is_never_buffered() {
    let f = fixture().await;
    let waiting = waiter(&f).await;
    let first = signal_with_start(&f, waiting, "cart-46", b"{}", &key())
        .await
        .expect("started");
    cancel(&f, first.run.info.run_id).await;

    let refused =
        f.w.runs
            .signal_with_start(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: waiting,
                    version: None,
                    input: br#"{"k": "s"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business("cart-46", ReusePolicy::RejectDuplicate),
                SignalAtStart {
                    name: SIGNAL,
                    payload: b"{}",
                },
                &ctx(),
            )
            .await;

    assert!(
        matches!(
            refused,
            Err(AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_REUSE_REJECTED",
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(runs_of(&f, "cart-46").await.len(), 1);
}

#[tokio::test]
async fn the_same_key_replays_the_outcome_and_another_body_is_another_request() {
    let f = fixture().await;
    let waiting = waiter(&f).await;
    let k = key();

    let first = signal_with_start(&f, waiting, "cart-47", br#"{"n": 1}"#, &k)
        .await
        .expect("started and signalled");
    let replay = signal_with_start(&f, waiting, "cart-47", br#"{"n": 1}"#, &k)
        .await
        .expect("a retry of the same request");

    assert_eq!(replay.run.info.run_id, first.run.info.run_id);
    assert!(replay.started, "it says what the first request did");
    assert_eq!(nodes(&replay), nodes(&first));
    assert_eq!(runs_of(&f, "cart-47").await.len(), 1);

    let other = signal_with_start(&f, waiting, "cart-47", br#"{"n": 2}"#, &k).await;
    assert_eq!(other.err(), Some(AppError::IdempotencyKeyReused));
}

/// An ordinary start of `workflow` under `id`, with the same body a with-start request would carry.
async fn plain_start(
    f: &Fixture,
    workflow: WorkflowId,
    id: &str,
    key: &str,
) -> Result<sagat_application::runs::Started, AppError> {
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: br#"{"k": "s"}"#,
                correlation_id: None,
                deadline: None,
                key,
            },
            sagat_application::runs::StartOptions {
                business: Some(business(id, ReusePolicy::AllowDuplicate)),
                ..sagat_application::runs::StartOptions::default()
            },
            &ctx(),
        )
        .await
}

#[tokio::test]
async fn a_key_used_by_a_plain_start_is_not_a_with_start_request_and_the_other_way_round() {
    let f = fixture().await;
    let waiting = waiter(&f).await;

    // an ordinary start came first: a with-start under its key is another operation, not a retry that
    // would be answered as if it had delivered a signal
    let k = key();
    plain_start(&f, waiting, "cart-50", &k)
        .await
        .expect("started");
    let reused = signal_with_start(&f, waiting, "cart-50", b"{}", &k).await;
    assert_eq!(reused.err(), Some(AppError::IdempotencyKeyReused));

    // and a with-start came first: a plain start under its key is not a retry of it either
    let k = key();
    signal_with_start(&f, waiting, "cart-51", b"{}", &k)
        .await
        .expect("started and signalled");
    let reused = plain_start(&f, waiting, "cart-51", &k).await;
    assert_eq!(reused.err(), Some(AppError::IdempotencyKeyReused));
    assert_eq!(runs_of(&f, "cart-51").await.len(), 1);
}

#[tokio::test]
async fn it_needs_the_right_to_start_and_the_right_to_signal() {
    let f = fixture().await;
    let waiting = waiter(&f).await;
    let viewer = f.w.member("reader", Role::Viewer).await;

    let refused =
        f.w.runs
            .signal_with_start(
                &viewer,
                &StartRun {
                    namespace: "payments",
                    workflow: waiting,
                    version: None,
                    input: br#"{"k": "s"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business("cart-48", ReusePolicy::AllowDuplicate),
                SignalAtStart {
                    name: SIGNAL,
                    payload: b"{}",
                },
                &ctx(),
            )
            .await;

    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    assert_eq!(runs_of(&f, "cart-48").await.len(), 0);

    // A developer may start runs but not signal them: starting and signalling is two rights, and
    // having the first does not give the second.
    let developer = f.w.member("builder", Role::Developer).await;
    let refused =
        f.w.runs
            .signal_with_start(
                &developer,
                &StartRun {
                    namespace: "payments",
                    workflow: waiting,
                    version: None,
                    input: br#"{"k": "s"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business("cart-49", ReusePolicy::AllowDuplicate),
                SignalAtStart {
                    name: SIGNAL,
                    payload: b"{}",
                },
                &ctx(),
            )
            .await;
    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    assert_eq!(
        runs_of(&f, "cart-49").await.len(),
        0,
        "nothing was started for a caller who may not signal"
    );
}

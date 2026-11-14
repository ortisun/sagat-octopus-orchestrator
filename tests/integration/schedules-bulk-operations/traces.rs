//! The trace of a run the engine starts (`D2084`, `D2087`, `NFR-018-006`): a run that no request started (the pass of a schedule, the
//! completion of a pending reset, the rerun of a batch job) has no trace to take from a request, and the telemetry projector does not
//! tell a run with no trace id to the collector. Each of them begins one, a random 32-hex id that names nothing of the customer's; the
//! trace of a request that starts a run (trigger-now) is kept, not replaced. Real PostgreSQL, the real passes.
//!
//! What is **not** given a trace (`G2032`): the operations the engine makes on a run that already has one (the cancellation a
//! `CANCEL_OTHER` schedule pays, the item actions that are not a rerun, the cancellation of the source of a reset) keep the audit
//! record without a trace id; the run they act on has its own trace, and the OTel evidence of the operation is that run's.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::identity::RequestContext;
use sagat_application::reset_completion::Completed;
use sagat_application::schedule_firing::TriggerRequest;
use sagat_domain::batch::{Action, Selector};
use sagat_telemetry::TraceId;
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::batch_executor::{
    create, due, executor, finish_the_only_open_run, live, runs_started, select, settle, write,
};
use crate::cancel_source::{asking, completer, due_of, end, made_from, open_chain, request};
use crate::firing::{fixture, hourly};

async fn trace_of(f: &crate::firing::Fixture, run: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT trace_id FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the run")
}

/// A valid W3C trace id: 32 lowercase hex characters, not all zero.
fn assert_a_trace(what: &str, trace: Option<String>) -> String {
    let trace = trace
        .unwrap_or_else(|| panic!("{what} has no trace id: it is never told to the collector"));
    assert_eq!(trace.len(), 32, "{what}: {trace}");
    assert!(
        trace
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{what}: lowercase hex: {trace}"
    );
    assert!(
        TraceId::parse(&trace).is_some(),
        "{what}: a valid trace id: {trace}"
    );
    trace
}

#[tokio::test]
async fn the_runs_a_schedule_starts_each_begin_a_trace_and_trigger_now_keeps_the_trace_of_its_request()
 {
    let f = fixture().await;
    let schedule = f
        .create("traces", &hourly(), &json!({"overlap": "ALLOW_ALL"}), None)
        .await;
    // two times of the schedule: two runs, two traces of their own
    let mut traces = Vec::new();
    for _ in 0..2 {
        f.clock.advance(Duration::hours(1) + Duration::seconds(1));
        assert!(f.drain(&schedule).await >= 1);
        let run = *f.runs(&schedule).await.last().expect("a run");
        traces.push(assert_a_trace(
            "a run the schedule pass started",
            trace_of(&f, run).await,
        ));
    }
    assert_ne!(traces[0], traces[1], "a trace for each run");

    // trigger-now is a request: the trace it carries is the one of its run
    let known = TraceId::generate().to_string();
    let action = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "traces-trigger-now",
            },
            &RequestContext {
                trace_id: Some(known.clone()),
                span_id: None,
            },
        )
        .await
        .expect("trigger");
    assert_eq!(action.outcome, "STARTED");
    let triggered: Uuid = sqlx::query_scalar(
        "SELECT run_id FROM schedule_actions WHERE schedule_id = $1 AND kind = 'TRIGGERED'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the run of the trigger");
    assert_eq!(
        trace_of(&f, triggered).await.as_deref(),
        Some(known.as_str()),
        "the trace of the request is kept"
    );
}

#[tokio::test]
async fn the_run_the_completion_of_a_pending_reset_makes_begins_a_trace() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), "traces-reset-key")
        .await
        .expect("the request");
    end(&f, &open).await;

    let done = completer(&f)
        .step(&due_of(&f, made.reset.id), None)
        .await
        .expect("the completion");
    assert_eq!(done, Completed::Created);

    let new_run = made_from(&f, open.run).await[0];
    assert_a_trace(
        "the run the completion of a pending reset made",
        trace_of(&f, new_run).await,
    );
}

#[tokio::test]
async fn the_run_a_batch_rerun_makes_begins_a_trace() {
    let f = fixture().await;
    let source = runs_started(&f, 1).await[0];
    finish_the_only_open_run(&f.w).await;
    live(&f);
    let exec = executor(&f);
    let job = create(
        &f,
        write(
            Action::Rerun,
            Selector::run_ids(vec![source]).expect("selector"),
        ),
        "traces-rerun",
    )
    .await;
    select(&exec, &job).await;
    settle(&exec, &job).await;
    let _ = due(&job);

    let new_run: Uuid = sqlx::query_scalar("SELECT new_run_id FROM batch_items WHERE job_id = $1")
        .bind(job.id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("a rerun says which run it made");
    assert_a_trace("the run a batch rerun made", trace_of(&f, new_run).await);
}

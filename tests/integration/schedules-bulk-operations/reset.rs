//! Reset to a point (`T018-053`, `T018-054`, `FR-018-035`..`FR-018-037`): a run is created from the prefix of an
//! ended one, its journal re-emits the events `1..=N` as the source stored them, marked `restored`, then
//! records the reset and goes on from there. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::execution::child_run_id;
use sagat_application::runs::{RESET_PREFIX_EVENTS, RerunBusinessId, ResetRequest};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{ResetReapply, RunId, RunStatus, StepStatus};
use sagat_domain::identity::Role;
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};

use crate::business_ids::key;
use crate::firing::{Fixture, fixture};
use crate::support::engine::{activity, ctx, document, with};

/// A workflow of three activities, one after the other.
async fn chain(f: &Fixture) -> WorkflowId {
    f.w.publish(document(
        &format!("reset-chain-{}", uuid::Uuid::now_v7()),
        "a",
        vec![
            with(activity("a"), "next", json!("b")),
            with(activity("b"), "next", json!("c")),
            activity("c"),
        ],
    ))
    .await
    .0
}

/// Runs the activities named, in order, to completion with an output each; the run ends when they are done.
async fn finish(f: &Fixture, run: RunId, nodes: &[&str]) {
    let activities: Vec<String> = nodes.iter().map(|node| format!("act.{node}")).collect();
    let names: Vec<&str> = activities.iter().map(String::as_str).collect();
    let session = f.w.session(&names).await;
    for node in nodes {
        let task = f.w.one_task_of(&session, &format!("act.{node}")).await;
        f.w.complete(&session, &task, &json!({ "done": node }))
            .await
            .expect("the step completes");
    }
    let _ = run;
}

async fn ended_chain(f: &Fixture) -> RunId {
    let workflow = chain(f).await;
    let run = f.w.start(workflow, &json!({"k": 1})).await;
    finish(f, run, &["a", "b", "c"]).await;
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);
    run
}

fn request(sequence: u64, acknowledge: bool) -> ResetRequest<'static> {
    ResetRequest {
        sequence,
        reason: "the third step used a wrong price list",
        acknowledge_reexecution: acknowledge,
        business: RerunBusinessId::Keep,
        reapply: ResetReapply::None,
    }
}

/// `(seq, event_type, actor, payload digest, restored, occurred at, trace)` of a row of the journal.
type Row = (
    i64,
    String,
    String,
    Option<Vec<u8>>,
    Option<String>,
    time::OffsetDateTime,
    Option<String>,
);

/// Every row of a run, in order.
async fn rows(f: &Fixture, run: RunId) -> Vec<Row> {
    sqlx::query_as(
        "SELECT seq, event_type, actor, payload_digest, detail->>'restored', occurred_at, trace_id \
         FROM run_events WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the journal")
}

#[tokio::test]
async fn a_run_is_reset_to_a_point_and_goes_on_from_there_with_the_history_it_had() {
    let f = fixture().await;
    let source = ended_chain(&f).await;

    // the points: the start and the completion of each step
    let points =
        f.w.runs
            .reset_points(&f.w.admin, source, &ctx())
            .await
            .expect("points");
    let sequences: Vec<u64> = points.iter().map(|point| point.sequence).collect();
    let source_rows = rows(&f, source).await;
    let completed: Vec<u64> = source_rows
        .iter()
        .filter(|row| row.1 == "step.completed")
        .map(|row| u64::try_from(row.0).expect("small"))
        .collect();
    let mut expected = vec![1];
    expected.extend(&completed);
    assert_eq!(sequences, expected);

    // the preview at the completion of `a`: `a` is restored, `b` and `c` run again
    let after_a = completed[0];
    let plan =
        f.w.runs
            .reset_preview(&f.w.admin, source, after_a, &ctx())
            .await
            .expect("a plan");
    assert_eq!(
        plan.restored
            .iter()
            .map(|step| step.node.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(
        plan.reexecuted
            .iter()
            .map(|step| step.node.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert!(plan.needs_acknowledgement());

    // steps that completed in the source run again: the caller says it knows
    let refused =
        f.w.runs
            .reset(&f.w.admin, source, request(after_a, false), &key(), &ctx())
            .await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                ..
            })
        ),
        "{refused:?}"
    );

    let reset =
        f.w.runs
            .reset(&f.w.admin, source, request(after_a, true), &key(), &ctx())
            .await
            .expect("the reset");
    let new = reset.info.run_id;
    assert_ne!(new, source);
    assert_eq!(reset.info.rerun_of, Some(source));
    assert_eq!(reset.status, RunStatus::Running);

    // the journal: the source's events 1..=N as they were, then the reset and the release of `b`
    let new_rows = rows(&f, new).await;
    let n = usize::try_from(after_a).expect("small");
    for (restored, original) in new_rows.iter().zip(&source_rows).take(n) {
        assert_eq!(restored.0, original.0, "the sequence");
        assert_eq!(restored.1, original.1, "the type of {}", original.0);
        assert_eq!(restored.2, original.2, "who did {}", original.0);
        assert_eq!(restored.3, original.3, "the payload of {}", original.0);
        assert_eq!(restored.5, original.5, "when {} happened", original.0);
        assert_eq!(restored.6, original.6, "the trace of {}", original.0);
        assert_eq!(
            restored.4.as_deref(),
            Some("true"),
            "{} is marked restored",
            original.0
        );
    }
    // history has none of the effects of the first time: only `b`, released by the reset, is announced
    let announced: Vec<String> = sqlx::query_scalar(
        "SELECT routing_key FROM event_outbox WHERE aggregate_id = $1 AND event_type = 'task.ready'",
    )
    .bind(new.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the outbox");
    assert_eq!(announced, ["act.b"]);
    let after: Vec<&str> = new_rows[n..].iter().map(|row| row.1.as_str()).collect();
    assert_eq!(after, ["run.reset", "step.routed", "step.released"]);
    assert!(
        new_rows[n..].iter().all(|row| row.4.is_none()),
        "what the run does itself is not restored"
    );
    let reset_sequence: Option<i64> =
        sqlx::query_scalar("SELECT reset_sequence FROM runs WHERE id = $1")
            .bind(new.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the column");
    assert_eq!(reset_sequence, Some(i64::try_from(after_a).expect("small")));
    let origin: Value = sqlx::query_scalar(
        "SELECT detail->'reset_of' FROM run_events WHERE run_id = $1 AND seq = 1",
    )
    .bind(new.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the first row");
    assert_eq!(origin["source_run_id"], source.to_string());
    assert_eq!(origin["source_sequence"], after_a);

    // `a` is done as it was, `b` waits for a worker; the projection equals a replay of the journal
    assert_eq!(f.w.step_status(new, "a").await, Some(StepStatus::Succeeded));
    assert_eq!(f.w.step_status(new, "b").await, Some(StepStatus::Ready));
    f.w.assert_consistent(new).await;
    // the restored step holds the output the source gave it, the very same content
    let output = |run: RunId| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<Vec<u8>>>(
                "SELECT output_digest FROM steps WHERE run_id = $1 AND node = 'a'",
            )
            .bind(run.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("the step")
        }
    };
    assert!(output(source).await.is_some());
    assert_eq!(output(new).await, output(source).await);

    // and the run goes on: `b` and `c` run again, with their own outputs, and the run ends
    finish(&f, new, &["b", "c"]).await;
    assert_eq!(f.w.run_status(new).await, RunStatus::Succeeded);
    f.w.assert_consistent(new).await;
    // the source is what it was
    assert_eq!(rows(&f, source).await, source_rows);
    f.w.assert_consistent(source).await;
}

#[tokio::test]
async fn a_reset_to_the_start_is_a_run_of_its_own_and_the_refusals_are_typed() {
    let f = fixture().await;
    let source = ended_chain(&f).await;

    // the start restores nothing but the start: every step runs again
    let fresh =
        f.w.runs
            .reset(&f.w.admin, source, request(1, true), &key(), &ctx())
            .await
            .expect("a reset to the start");
    assert_eq!(
        f.w.step_status(fresh.info.run_id, "a").await,
        Some(StepStatus::Ready)
    );
    f.w.assert_consistent(fresh.info.run_id).await;

    // a sequence that is not a point of the journal
    let not_a_point =
        f.w.runs
            .reset(&f.w.admin, source, request(2, true), &key(), &ctx())
            .await;
    assert!(
        matches!(
            not_a_point,
            Err(AppError::ResetRefused {
                reason: "RESET_POINT_NOT_ELIGIBLE",
                ..
            })
        ),
        "{not_a_point:?}"
    );

    // a run that is still open is not reset (REQUIRE_TERMINAL)
    let workflow = chain(&f).await;
    let open = f.w.start(workflow, &json!({})).await;
    let refused =
        f.w.runs
            .reset(&f.w.admin, open, request(1, true), &key(), &ctx())
            .await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_SOURCE_NOT_TERMINAL",
                ..
            })
        ),
        "{refused:?}"
    );

    // a justification, and a delivery after the point this version does not make
    let mut no_reason = request(1, true);
    no_reason.reason = "short";
    assert!(matches!(
        f.w.runs
            .reset(&f.w.admin, source, no_reason, &key(), &ctx())
            .await,
        Err(AppError::NotAllowed(_))
    ));
    let mut signals = request(1, true);
    signals.reapply = ResetReapply::Signals;
    assert!(matches!(
        f.w.runs
            .reset(&f.w.admin, source, signals, &key(), &ctx())
            .await,
        Err(AppError::NotAllowed(_))
    ));

    // a viewer may read runs and may not reset them; a run of another tenant is not there
    let viewer = f.w.member("viewer-of-resets", Role::Viewer).await;
    assert!(matches!(
        f.w.runs.reset_points(&viewer, source, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        f.w.runs
            .reset(&f.w.other_admin, source, request(1, true), &key(), &ctx())
            .await,
        Err(AppError::NotFound)
    ));
}

#[tokio::test]
async fn the_declared_state_and_the_output_of_a_signal_in_the_prefix_are_restored() {
    use crate::retained_arguments::{apply, arguments, publish};
    let f = fixture().await;
    let workflow = publish(&f, false).await;
    // an Update at the start, a signal that wakes the wait, and `notify` runs
    let source = apply(
        &f,
        workflow,
        "reset-state",
        "update-key-0001",
        &arguments(true, 7),
    )
    .await;
    f.w.runs
        .signal(
            &f.w.admin,
            source,
            "payment.confirmed",
            br#"{"paid":true}"#,
            &key(),
            &ctx(),
        )
        .await
        .expect("the signal");
    finish(&f, source, &["notify"]).await;
    assert_eq!(f.w.run_status(source).await, RunStatus::Succeeded);

    let points =
        f.w.runs
            .reset_points(&f.w.admin, source, &ctx())
            .await
            .expect("points");
    let last = points.last().expect("a point").sequence;
    let source_rows = rows(&f, source).await;
    assert!(
        source_rows
            .iter()
            .any(|row| row.1 == "state.updated" && row.0 < i64::try_from(last).expect("small")),
        "the Update is in the prefix"
    );

    let reset =
        f.w.runs
            .reset(&f.w.admin, source, request(last, true), &key(), &ctx())
            .await
            .expect("the reset");
    let new = reset.info.run_id;
    f.w.assert_consistent(new).await;

    // the declared state is the one the source had at the point: the Update's revision and digest
    let state = |run: RunId| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_as::<_, (Vec<u8>, i64, i64)>(
                "SELECT state_digest, state_revision, last_changed_run_version \
                 FROM workflow_operation_states WHERE run_id = $1",
            )
            .bind(run.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("the declared state")
        }
    };
    assert_eq!(state(new).await, state(source).await);
    // the wait kept the payload of the signal that woke it, as the source's row named it
    let named = |rows: &[Row]| {
        rows.iter()
            .find(|row| row.1 == "wait.satisfied")
            .map(|row| row.3.clone())
    };
    let new_rows = rows(&f, new).await;
    assert!(named(&source_rows).flatten().is_some());
    assert_eq!(named(&new_rows), named(&source_rows));
}

#[tokio::test]
async fn the_same_key_is_the_same_reset_and_another_point_under_it_is_another_request() {
    let f = fixture().await;
    let source = ended_chain(&f).await;
    let completed = rows(&f, source)
        .await
        .iter()
        .filter(|row| row.1 == "step.completed")
        .map(|row| u64::try_from(row.0).expect("small"))
        .collect::<Vec<_>>();
    let key = key();

    let first =
        f.w.runs
            .reset(
                &f.w.admin,
                source,
                request(completed[0], true),
                &key,
                &ctx(),
            )
            .await
            .expect("first");
    let again =
        f.w.runs
            .reset(
                &f.w.admin,
                source,
                request(completed[0], true),
                &key,
                &ctx(),
            )
            .await
            .expect("a retry");
    assert_eq!(
        first.info.run_id, again.info.run_id,
        "a retry is answered with the run it created"
    );
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs, 1);

    let other =
        f.w.runs
            .reset(
                &f.w.admin,
                source,
                request(completed[1], true),
                &key,
                &ctx(),
            )
            .await;
    assert_eq!(
        other.err(),
        Some(AppError::IdempotencyKeyReused),
        "the key of another point is not a retry"
    );
}

fn reason_of<T: std::fmt::Debug>(answer: Result<T, AppError>) -> (&'static str, Option<String>) {
    match answer {
        Err(AppError::ResetRefused { reason, event_type }) => (reason, event_type),
        other => panic!("a reset refusal was expected: {other:?}"),
    }
}

#[tokio::test]
async fn a_sequence_past_the_ceiling_is_too_long_and_the_ceiling_itself_is_not() {
    let f = fixture().await;
    let source = ended_chain(&f).await;
    let ceiling = u64::try_from(RESET_PREFIX_EVENTS).expect("small");

    // past the ceiling, for the preview and for the reset, whatever the journal holds
    let past = ceiling + 1;
    assert_eq!(
        reason_of(
            f.w.runs
                .reset_preview(&f.w.admin, source, past, &ctx())
                .await
        ),
        ("RESET_PREFIX_TOO_LONG", None)
    );
    assert_eq!(
        reason_of(
            f.w.runs
                .reset(&f.w.admin, source, request(past, true), &key(), &ctx())
                .await
        ),
        ("RESET_PREFIX_TOO_LONG", None)
    );
    // at the ceiling the sequence is allowed; this journal is shorter, so it is not a point
    assert_eq!(
        reason_of(
            f.w.runs
                .reset_preview(&f.w.admin, source, ceiling, &ctx())
                .await
        ),
        ("RESET_POINT_NOT_ELIGIBLE", None)
    );
    // and no run was created by any of them
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs, 0);
}

#[tokio::test]
async fn a_child_run_is_not_a_source_of_a_reset() {
    let f = fixture().await;
    let suffix = uuid::Uuid::now_v7();
    let kid = format!("reset-kid-{suffix}");
    f.w.publish(document(
        &kid,
        "kid",
        vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid" })],
    ))
    .await;
    let mother =
        f.w.publish(document(
            &format!("reset-mother-{suffix}"),
            "c",
            vec![
                with(
                    crate::support::engine::child_node("c", &kid, None),
                    "next",
                    json!("after"),
                ),
                activity("after"),
            ],
        ))
        .await
        .0;
    let parent = f.w.start(mother, &json!({})).await;
    let report = f.w.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    let child = child_run_id(parent, &NodeId::parse("c").expect("a node"));
    assert!(f.w.run_record(child).await.is_some(), "the child exists");

    // the refusal does not depend on the point, nor on whether the child has ended
    assert_eq!(
        reason_of(f.w.runs.reset_points(&f.w.admin, child, &ctx()).await),
        ("RESET_NOT_SUPPORTED_FOR_SOURCE", None)
    );
    assert_eq!(
        reason_of(
            f.w.runs
                .reset(&f.w.admin, child, request(1, true), &key(), &ctx())
                .await
        ),
        ("RESET_NOT_SUPPORTED_FOR_SOURCE", None)
    );
}

#[tokio::test]
async fn a_point_past_an_event_this_version_cannot_read_has_no_rule() {
    let f = fixture().await;
    let source = ended_chain(&f).await;
    let points: Vec<u64> =
        f.w.runs
            .reset_points(&f.w.admin, source, &ctx())
            .await
            .expect("points")
            .iter()
            .map(|point| point.sequence)
            .collect();
    let last_point = *points.last().expect("a point");

    // a later version wrote an event this one does not know, after everything the run did
    let last: i64 = sqlx::query_scalar("SELECT max(seq) FROM run_events WHERE run_id = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the last row");
    sqlx::query(
        "INSERT INTO run_events (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,$3,'future.reset.observed','{}','system:reset-test',clock_timestamp())",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(source.as_uuid())
    .bind(last + 1)
    .execute(&f.w.db.pool)
    .await
    .expect("an event of a newer kind");

    // the points before it are as they were
    let after: Vec<u64> =
        f.w.runs
            .reset_points(&f.w.admin, source, &ctx())
            .await
            .expect("points")
            .iter()
            .map(|point| point.sequence)
            .collect();
    assert_eq!(after, points);
    f.w.runs
        .reset_preview(&f.w.admin, source, last_point, &ctx())
        .await
        .expect("a point before the unknown event has a plan");

    // a prefix that would hold it has no rule to apply, and the answer names the type
    let beyond = u64::try_from(last + 1).expect("small");
    assert_eq!(
        reason_of(
            f.w.runs
                .reset_preview(&f.w.admin, source, beyond, &ctx())
                .await
        ),
        (
            "RESET_RULE_MISSING",
            Some("future.reset.observed".to_owned())
        )
    );
}

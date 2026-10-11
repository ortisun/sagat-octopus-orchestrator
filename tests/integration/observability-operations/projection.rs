//! A projection that disagrees with the journal (T008-030). The note a run leaves is written in the
//! transaction that appends to its journal, so the note cannot be ahead of the journal; but a
//! journal restored from a backup, or trimmed by a retention policy (G28, feature 010), can leave
//! one that is. The projector must settle such a note instead of asking for it again forever.

use sagat_application::telemetry::TelemetryStore;
use sagat_scheduler::{ProjectorSettings, TelemetryProjector};
use serde_json::json;

use crate::support::collector::Collector;
use crate::support::engine::{activity, document, world};

#[tokio::test]
async fn a_note_that_claims_more_than_the_journal_holds_is_settled_and_does_not_keep_the_projector_busy()
 {
    let w = world().await;
    let collector = Collector::start().await;
    let (wf, _) = w
        .publish(document("inconsistent", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    let projector = TelemetryProjector::new(
        w.store.clone(),
        Some(collector.client()),
        ProjectorSettings::default(),
    );
    assert_eq!(projector.run_once().await.runs, 1);
    let exported = collector.spans().len();
    assert!(exported > 0);
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 0);

    // The note is made to claim entries the journal does not have.
    sqlx::query(
        "UPDATE telemetry_pending SET head_seq = head_seq + 5, \
                behind_since = clock_timestamp() - interval '10 seconds' WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("corrupt the note");
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 1);

    let settling = projector.run_once().await;
    assert!(!settling.failed, "{settling:?}");
    assert_eq!(
        w.store.backlog().await.expect("backlog").runs,
        0,
        "the note is settled to what the journal holds"
    );
    assert_eq!(
        collector.spans().len(),
        exported,
        "nothing is exported twice for it"
    );
    let idle = projector.run_once().await;
    assert!(
        !idle.did_work(),
        "the loop would otherwise run flat out on it: {idle:?}"
    );

    // And a run that really does move afterwards is projected as ever.
    let (wf2, _) = w
        .publish(document("after-inconsistent", "a", vec![activity("a")]))
        .await;
    w.start(wf2, &json!({})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(projector.run_once().await.runs, 1);
    assert!(collector.spans().len() > exported);
}

#[tokio::test]
async fn a_trace_the_sampler_does_not_keep_is_not_exported_and_its_progress_moves() {
    let w = world().await;
    let collector = Collector::start().await;
    let (wf, _) = w
        .publish(document("sampled-out", "a", vec![activity("a")]))
        .await;
    w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");

    let nothing = TelemetryProjector::new(
        w.store.clone(),
        Some(collector.client()),
        ProjectorSettings {
            sample_ratio: 0.0,
            ..ProjectorSettings::default()
        },
    );
    let round = nothing.run_once().await;
    assert_eq!(round.runs, 1, "{round:?}");
    assert_eq!(round.spans, 0, "{round:?}");
    assert!(
        collector.spans().is_empty(),
        "a trace that is not kept leaves the platform in no span"
    );
    assert_eq!(
        w.store.backlog().await.expect("backlog").runs,
        0,
        "and the run is not asked for again"
    );

    // The same projector settings with everything kept export the next run.
    let (wf2, _) = w
        .publish(document("sampled-in", "a", vec![activity("a")]))
        .await;
    w.start(wf2, &json!({})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    let everything = TelemetryProjector::new(
        w.store.clone(),
        Some(collector.client()),
        ProjectorSettings::default(),
    );
    assert_eq!(everything.run_once().await.runs, 1);
    assert!(!collector.spans().is_empty());
}

#[tokio::test]
async fn a_collector_that_wants_other_credentials_delays_the_spans_and_loses_none_of_them() {
    let w = world().await;
    let collector = Collector::start().await;
    let (wf, _) = w
        .publish(document("unauthorised", "a", vec![activity("a")]))
        .await;
    w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    let projector = TelemetryProjector::new(
        w.store.clone(),
        Some(collector.client()),
        ProjectorSettings::default(),
    );

    // An expired token is the operator's to fix; the spans wait in the journal meanwhile.
    collector.answer_with(&[401, 401, 401, 401, 401, 401, 401, 401]);
    let round = projector.run_once().await;
    assert!(round.failed, "{round:?}");
    assert_eq!(round.runs, 0, "{round:?}");
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 1);
    assert!(collector.spans().is_empty());

    // Once it is fixed the same run is projected.
    collector.answer_with(&[]);
    let round = projector.run_once().await;
    assert_eq!(round.runs, 1, "{round:?}");
    assert!(!collector.spans().is_empty());
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 0);
}

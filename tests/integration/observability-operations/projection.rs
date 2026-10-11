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

// ------------------------------------------------------------------ a second projector

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use sagat_application::error::StoreError;
use sagat_application::telemetry::{Backlog, Pending, RunTelemetry, ToProject};
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use sagat_infra_postgres::PgRunStore;

/// The real store, behind which another projector can have been at work, and whose lease can be
/// lost in the middle of a round.
#[derive(Clone)]
struct Contested {
    inner: PgRunStore,
    /// Another projector marks the run just before this one does.
    other_marks_first: bool,
    /// The lease is held for this many calls and then lost (`u32::MAX`: never).
    lease_holds_for: u32,
    leases_asked: Arc<AtomicU32>,
}

impl Contested {
    fn new(inner: PgRunStore) -> Self {
        Self {
            inner,
            other_marks_first: false,
            lease_holds_for: u32::MAX,
            leases_asked: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl TelemetryStore for Contested {
    async fn acquire_projector(&self) -> Result<bool, StoreError> {
        let asked = self.leases_asked.fetch_add(1, Ordering::SeqCst) + 1;
        if asked > self.lease_holds_for {
            return Ok(false);
        }
        self.inner.acquire_projector().await
    }

    async fn pending_runs(&self, limit: u32) -> Result<Vec<Pending>, StoreError> {
        self.inner.pending_runs(limit).await
    }

    async fn load_for_projection(
        &self,
        pending: &Pending,
    ) -> Result<Option<ToProject>, StoreError> {
        self.inner.load_for_projection(pending).await
    }

    async fn mark_projected(&self, pending: &Pending, upto: u64) -> Result<bool, StoreError> {
        if self.other_marks_first {
            self.inner.mark_projected(pending, upto).await?;
        }
        self.inner.mark_projected(pending, upto).await
    }

    async fn run_telemetry(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<RunTelemetry>, StoreError> {
        self.inner.run_telemetry(tenant, run).await
    }

    async fn backlog(&self) -> Result<Backlog, StoreError> {
        self.inner.backlog().await
    }
}

async fn finished_runs(w: &crate::support::engine::World, names: &[&str]) {
    let session = w.session(&["act.a"]).await;
    for name in names {
        let (wf, _) = w.publish(document(name, "a", vec![activity("a")])).await;
        w.start(wf, &json!({})).await;
        let task = w.one_task(&session).await;
        w.complete(&session, &task, &json!({"ok": true}))
            .await
            .expect("complete");
    }
}

#[tokio::test]
async fn a_run_another_projector_took_in_the_meantime_is_not_counted_twice() {
    let w = world().await;
    finished_runs(&w, &["contested"]).await;
    let mut store = Contested::new(w.store.clone());
    store.other_marks_first = true;
    let projector = TelemetryProjector::new(store, None, ProjectorSettings::default());
    let round = projector.run_once().await;
    assert_eq!(
        round.runs, 0,
        "its measurements were made by whoever moved the note first: {round:?}"
    );
    assert_eq!(round.skipped, 1, "{round:?}");
}

#[tokio::test]
async fn a_projector_that_loses_its_lease_between_two_batches_stops_before_the_second() {
    let w = world().await;
    let collector = Collector::start().await;
    finished_runs(&w, &["first", "second"]).await;
    let mut store = Contested::new(w.store.clone());
    // The lease is asked for when the round starts and before each batch is handed over.
    store.lease_holds_for = 2;
    let projector = TelemetryProjector::new(
        store,
        Some(collector.client()),
        ProjectorSettings {
            // One run to a batch.
            batch_spans: 1,
            ..ProjectorSettings::default()
        },
    );
    let round = projector.run_once().await;
    assert_eq!(round.runs, 1, "the first batch was taken: {round:?}");
    assert!(
        round.passive,
        "and the round ended when the lease was lost: {round:?}"
    );
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 1);
}

#[tokio::test]
async fn a_run_projected_in_part_is_behind_since_its_first_change_not_taken_and_not_since_before() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("partial", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    let note = w
        .store
        .pending_runs(10)
        .await
        .expect("pending")
        .into_iter()
        .find(|n| n.run_id == run)
        .expect("the note");
    assert!(note.head_seq >= 3);
    // The projector took the first two entries and the others came in meanwhile.
    sqlx::query("UPDATE telemetry_pending SET behind_since = clock_timestamp() - interval '1 hour' WHERE run_id = $1")
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("make the lag old");
    assert!(w.store.mark_projected(&note, 2).await.expect("mark"));
    let (applied, behind_for): (i64, f64) = sqlx::query_as(
        "SELECT applied_seq, extract(epoch FROM clock_timestamp() - behind_since)::float8 \
         FROM telemetry_pending WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("note");
    assert_eq!(applied, 2);
    assert!(
        behind_for < 60.0,
        "the third entry was written seconds ago; the lag is not an hour: {behind_for}"
    );
    // Marking the same place again moves nothing.
    assert!(!w.store.mark_projected(&note, 2).await.expect("mark again"));
}

//! What satisfied a wait (`T018-043`, `FR-018-039`): the `wait.satisfied` row of the journal says `signal`,
//! `event` or `signal_with_start`, so a reset can tell the waits a caller satisfied, which it may deliver
//! again, from those an event satisfied, which it may not. Real PostgreSQL, the real services.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::execution::{CorrelationKey, RunId, RunStatus, StepStatus};
use sagat_domain::workflow::EventType;
use serde_json::json;

use crate::business_ids::key;
use crate::firing::{Fixture, fixture};
use crate::support::engine::ctx;
use crate::with_start::{SIGNAL, business, signal_with_start, waiter};
use sagat_application::runs::StartRun;
use sagat_domain::business_ids::ReusePolicy;

/// The `via` of every `wait.satisfied` row of the run, in journal order; `None` for a row that does not say.
async fn vias(f: &Fixture, run: RunId) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT detail->>'via' FROM run_events \
         WHERE run_id = $1 AND event_type = 'wait.satisfied' ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the journal")
}

/// A run of the waiter, started by an ordinary start.
async fn waiting_run(f: &Fixture) -> RunId {
    let workflow = waiter(f).await;
    let run = f.w.start(workflow, &json!({"k": "s"})).await;
    assert_eq!(f.w.run_status(run).await, RunStatus::Waiting);
    run
}

#[tokio::test]
async fn a_signal_to_the_run_is_recorded_as_a_signal() {
    let f = fixture().await;
    let run = waiting_run(&f).await;

    f.w.runs
        .signal(&f.w.admin, run, SIGNAL, b"{}", &key(), &ctx())
        .await
        .expect("signalled");

    assert_eq!(vias(&f, run).await, [Some("signal".to_owned())]);
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn a_published_event_is_recorded_as_an_event() {
    let f = fixture().await;
    let run = waiting_run(&f).await;

    f.w.events
        .publish(
            &f.w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse("confirmed-1").expect("id")),
                event_type: EventType::parse(SIGNAL).expect("type"),
                subject: None,
                correlation_key: Some(CorrelationKey::parse("s").expect("key")),
                occurred_at: None,
                data: b"{}".to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
    let round = f.w.dispatcher.tick().await;

    assert_eq!(round.steps_woken, 1);
    assert_eq!(f.w.step_status(run, "w").await, Some(StepStatus::Succeeded));
    assert_eq!(vias(&f, run).await, [Some("event".to_owned())]);
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn the_signal_of_a_signal_with_start_is_recorded_as_such_whether_it_creates_the_run_or_finds_it()
 {
    let f = fixture().await;
    let workflow = waiter(&f).await;

    // the run does not exist: created, and signalled in the transaction that creates it
    let created = signal_with_start(&f, workflow, "cart-via", b"{}", &key())
        .await
        .expect("started and signalled");
    assert!(created.started);
    assert_eq!(
        vias(&f, created.run.info.run_id).await,
        [Some("signal_with_start".to_owned())]
    );

    // the run exists and waits: the same request signals it, and says so the same way
    let holder =
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
                    key: &key(),
                },
                sagat_application::runs::StartOptions {
                    business: Some(business("cart-via-open", ReusePolicy::AllowDuplicate)),
                    ..sagat_application::runs::StartOptions::default()
                },
                &ctx(),
            )
            .await
            .expect("the holder")
            .run
            .info
            .run_id;
    let served = signal_with_start(&f, workflow, "cart-via-open", b"{}", &key())
        .await
        .expect("signalled");
    assert!(!served.started);
    assert_eq!(
        vias(&f, holder).await,
        [Some("signal_with_start".to_owned())]
    );
}

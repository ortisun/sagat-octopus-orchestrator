//! The telemetry of a reset run (`T018-058`, `FR-018-037`, `FR-018-044`): the projector tells what the new run
//! did and not what it copied. The attempts of the restored prefix were told by the source, once; the new run
//! has the span of its own, and the span links the one of its source. Real PostgreSQL, the real projector, a
//! real OTLP collector.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_scheduler::{ProjectorSettings, TelemetryProjector};

use crate::business_ids::key;
use crate::firing::fixture;
use crate::reset::{ended_chain, finish, request};
use crate::support::collector::Collector;

#[tokio::test]
async fn the_attempts_of_the_restored_prefix_are_told_once_by_the_source_and_the_new_run_links_it()
{
    let f = fixture().await;
    let collector = Collector::start().await;
    let projector = TelemetryProjector::new(
        f.w.store.clone(),
        Some(collector.client()),
        ProjectorSettings::default(),
    );

    // the source tells its three attempts and its span
    let source = ended_chain(&f).await;
    for _ in 0..2 {
        projector.run_once().await;
    }
    assert_eq!(collector.named("step.attempt").len(), 3);
    let source_id = source.to_string();
    let source_span = collector
        .named("workflow.run")
        .into_iter()
        .find(|span| span.attr("sagat.run.id") == Some(source_id.as_str()))
        .expect("the span of the source");

    // a reset keeps the first two steps; the third runs again in the new run
    let completed: Vec<i64> = sqlx::query_scalar(
        "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' ORDER BY seq",
    )
    .bind(source.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the completions");
    let point = u64::try_from(completed[1]).expect("small");
    let made =
        f.w.runs
            .reset(
                &f.w.admin,
                source,
                request(point, true),
                &key(),
                &crate::support::engine::ctx(),
            )
            .await
            .expect("the reset");
    let new = made.run.info.run_id;
    finish(&f, new, &["c"]).await;
    for _ in 0..2 {
        projector.run_once().await;
    }

    // one more attempt was told, and it is the new run's own
    let attempts = collector.named("step.attempt");
    assert_eq!(
        attempts.len(),
        4,
        "the restored attempts are not told a second time: {}",
        attempts
            .iter()
            .map(|span| format!(
                "{}:{}",
                span.attr("sagat.run.id").unwrap_or("?"),
                span.attr("sagat.node").unwrap_or("?")
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let new_id = new.to_string();
    let of_new: Vec<_> = attempts
        .iter()
        .filter(|span| span.attr("sagat.run.id") == Some(new_id.as_str()))
        .collect();
    assert_eq!(of_new.len(), 1);
    assert_eq!(of_new[0].attr("sagat.node"), Some("c"));

    // its span is its own and points at the source's
    let new_span = collector
        .named("workflow.run")
        .into_iter()
        .find(|span| span.attr("sagat.run.id") == Some(new_id.as_str()))
        .expect("the span of the new run");
    assert_ne!(new_span.span_id, source_span.span_id);
    assert_eq!(
        new_span.links,
        [(source_span.trace_id.clone(), source_span.span_id.clone())],
        "the link to the span of the source"
    );
    assert!(
        collector
            .named("workflow.run")
            .iter()
            .find(|span| span.attr("sagat.run.id") == Some(source_id.as_str()))
            .expect("the source's span")
            .links
            .is_empty(),
        "the source links nothing"
    );
}

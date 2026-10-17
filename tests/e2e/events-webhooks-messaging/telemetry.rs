//! The event machinery is counted, and the counters name no tenant, event, run or endpoint
//! (NFR-004-OBS).

use sagat_telemetry::Metrics;
use serde_json::json;

use crate::support::worker::{platform, single_ir};
use crate::{SECRET, endpoint, engine, runs_of, webhook, within};

/// The value of the series `name` whose labels include all of `labels`, summed.
fn counted(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(k, v)| line.contains(&format!("{k}=\"{v}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

#[tokio::test]
async fn events_webhooks_and_dispatches_are_counted_without_naming_anyone() {
    // The recorder is process-wide and other tests of this binary count too: every check is "at
    // least this many more than before", read from the same page.
    let metrics = Metrics::global();
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let endpoint_id = endpoint(&p, "billing", json!({})).await;
    let mut ir = single_ir("on-invoice", None);
    ir["triggers"] = json!([{"kind": "webhook", "endpoint": "billing"}]);
    let workflow = p.publish("on-invoice", &ir).await;
    let before = metrics.render();

    let body = json!({"invoice": "inv-secret-123"});
    assert_eq!(
        webhook(
            &p,
            &endpoint_id,
            "evt-telemetry-1",
            "invoice.paid",
            &body,
            SECRET
        )
        .await
        .0,
        202
    );
    assert_eq!(
        webhook(
            &p,
            &endpoint_id,
            "evt-telemetry-1",
            "invoice.paid",
            &body,
            SECRET
        )
        .await
        .0,
        202
    );
    assert_eq!(
        webhook(
            &p,
            &endpoint_id,
            "evt-telemetry-2",
            "invoice.paid",
            &body,
            "wrong-secret-wrong-secret-0000"
        )
        .await
        .0,
        401
    );
    within("the run", 30, || async {
        (runs_of(&p, &workflow).await.len() == 1).then_some(())
    })
    .await;
    // The run commits inside the dispatch; the event is marked dispatched and counted only after,
    // so the run can be visible before the counter moves.
    let dispatched = || {
        counted(
            &metrics.render(),
            "events_dispatched_total",
            &[("outcome", "dispatched")],
        )
    };
    let dispatched_before = counted(
        &before,
        "events_dispatched_total",
        &[("outcome", "dispatched")],
    );
    within("the dispatch count", 30, || async {
        (dispatched() >= dispatched_before + 1.0).then_some(())
    })
    .await;

    let after = metrics.render();
    let delta = |name: &str, labels: &[(&str, &str)]| {
        counted(&after, name, labels) - counted(&before, name, labels)
    };
    assert!(
        delta(
            "events_received_total",
            &[("kind", "webhook"), ("outcome", "accepted")]
        ) >= 1.0
    );
    assert!(
        delta(
            "events_received_total",
            &[("kind", "webhook"), ("outcome", "duplicate")]
        ) >= 1.0
    );
    assert!(delta("webhook_rejected_total", &[("reason", "bad_signature")]) >= 1.0);
    assert!(delta("trigger_events_total", &[("outcome", "run_started")]) >= 1.0);
    assert!(delta("events_dispatched_total", &[("outcome", "dispatched")]) >= 1.0);

    // No series names a tenant, an endpoint, an event or what the event carried.
    for needle in [
        "evt-telemetry",
        "inv-secret-123",
        "billing",
        &endpoint_id,
        &p.tenant.to_string(),
    ] {
        assert!(
            !after.contains(needle),
            "the metrics page mentions {needle}"
        );
    }
}

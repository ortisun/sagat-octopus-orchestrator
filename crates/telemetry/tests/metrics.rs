//! The Prometheus recorder forgets series that stop being updated (second security review,
//! finding 1: label values chosen by an anonymous client must not live in memory forever).

use std::time::Duration;

use sagat_telemetry::Metrics;

#[test]
fn the_process_recorder_expires_idle_series_after_a_bounded_time() {
    assert!(
        Metrics::IDLE_TIMEOUT >= Duration::from_secs(60)
            && Metrics::IDLE_TIMEOUT <= Duration::from_secs(3600),
        "long enough to survive scrape gaps, short enough to bound memory: {:?}",
        Metrics::IDLE_TIMEOUT
    );
}

#[test]
fn a_series_that_stops_being_updated_disappears_from_the_page() {
    let (metrics, recorder) = Metrics::detached(Duration::from_millis(150));
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!("probe_total", "label" => "stale").increment(1);
    });
    assert!(metrics.render().contains("probe_total{label=\"stale\"} 1"));

    std::thread::sleep(Duration::from_millis(400));
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!("probe_total", "label" => "fresh").increment(1);
    });
    let page = metrics.render();

    assert!(!page.contains("stale"), "{page}");
    assert!(page.contains("probe_total{label=\"fresh\"} 1"), "{page}");
}

#[test]
fn a_series_that_keeps_being_updated_is_kept() {
    let (metrics, recorder) = Metrics::detached(Duration::from_millis(300));
    for _ in 0..6 {
        metrics::with_local_recorder(&recorder, || {
            metrics::counter!("probe_total", "label" => "busy").increment(1);
        });
        std::thread::sleep(Duration::from_millis(100));
        metrics.prune_idle();
    }
    assert!(metrics.render().contains("probe_total{label=\"busy\"} 6"));
}

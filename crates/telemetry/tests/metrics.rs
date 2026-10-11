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

/// A latency or a lag recorded as a `*_seconds` histogram must be a histogram on the page: only
/// buckets can be added across replicas and turned into a percentile by the dashboard (FR-008-003).
/// A name that matched no bucket rule was rendered as a summary, whose quantiles cannot be merged.
#[test]
fn every_seconds_histogram_is_a_histogram_with_buckets_not_a_summary() {
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        metrics::histogram!("http_request_duration_seconds").record(0.2);
        metrics::histogram!("event_dispatch_lag_seconds").record(3.0);
        metrics::histogram!("step_schedule_latency_seconds").record(0.04);
        metrics::histogram!("script_run_seconds").record(1.5);
    });
    let page = metrics.render();
    for name in [
        "http_request_duration_seconds",
        "event_dispatch_lag_seconds",
        "step_schedule_latency_seconds",
        "script_run_seconds",
    ] {
        assert!(
            page.contains(&format!("{name}_bucket{{le=")),
            "{name} has no buckets:\n{page}"
        );
        assert!(
            !page.contains(&format!("{name}{{quantile=")),
            "{name} is a summary:\n{page}"
        );
    }
}

#[test]
fn a_lag_can_be_measured_in_minutes_and_a_latency_in_milliseconds() {
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        metrics::histogram!("event_dispatch_lag_seconds").record(400.0);
        metrics::histogram!("step_attempt_duration_seconds").record(0.003);
    });
    let page = metrics.render();
    assert!(
        page.contains("event_dispatch_lag_seconds_bucket{le=\"600\"} 1"),
        "{page}"
    );
    assert!(
        page.contains("step_attempt_duration_seconds_bucket{le=\"0.005\"} 1"),
        "{page}"
    );
}
